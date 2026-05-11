import json
from typing import Any


STYLE_CONSERVATIVE = "conservative"
MIN_LIVE_WAITS_FOR_RIICHI = 3
AUTO_RIICHI_ENABLED = False

HOT_PLAYER_WIN_COUNT = 2
HOT_PLAYER_FAST_RIICHI_COUNT = 2
HOT_PLAYER_DANGER_NOTE = "hot_opponent_conservative"
FOUR_PLAYER_LATE_DEFENCE_LEFT_TILES = 32
THREE_PLAYER_LATE_DEFENCE_LEFT_TILES = 24

ACTION_LABELS = [
    "1m", "2m", "3m", "4m", "5m", "6m", "7m", "8m", "9m",
    "1p", "2p", "3p", "4p", "5p", "6p", "7p", "8p", "9p",
    "1s", "2s", "3s", "4s", "5s", "6s", "7s", "8s", "9s",
    "E", "S", "W", "N", "P", "F", "C",
    "5mr", "5pr", "5sr",
    "reach", "chi_low", "chi_mid", "chi_high", "pon", "kan_select",
    "hora", "ryukyoku", "none",
]
DISCARD_ACTION_COUNT = 37


def _normalize(tile: str) -> str:
    return tile.replace("r", "")


def _to34(tile: str) -> int:
    tile = _normalize(tile)
    if tile == "?":
        return -1
    if tile in "ESWPFCDN":
        return 27 + "ESWPFCDN".index(tile)
    n = int(tile[0])
    suit = tile[1]
    return {"m": 0, "p": 9, "s": 18}[suit] + n - 1


def _count_tile(counts: list[int], tile: str) -> None:
    idx = _to34(tile)
    if 0 <= idx < len(counts):
        counts[idx] += 1


def _best_discard_from_meta(meta: dict[str, Any]) -> str | None:
    candidates = _discard_candidates_from_meta(meta)
    if not candidates:
        return None
    return max(candidates, key=lambda item: item[1])[0]


def _discard_candidates_from_meta(meta: dict[str, Any]) -> list[tuple[str, float]]:
    q_values = meta.get("q_values")
    mask_bits = meta.get("mask_bits")
    if not isinstance(q_values, list) or mask_bits is None:
        return []

    try:
        mask = int(mask_bits)
    except (TypeError, ValueError):
        return []

    candidates: list[tuple[str, float]] = []
    qi = 0
    for idx, label in enumerate(ACTION_LABELS):
        if not ((mask >> idx) & 1):
            continue
        if qi >= len(q_values):
            break
        try:
            score = float(q_values[qi])
        except (TypeError, ValueError):
            qi += 1
            continue
        qi += 1
        if idx >= DISCARD_ACTION_COUNT:
            continue
        candidates.append((label, score))
    return candidates


class ConservativePolicyGuard:
    """Post-process Mortal actions for house safety rules.

    The trained Mortal model is left untouched. This guard blocks reach
    recommendations that violate house safety rules, then falls back to the
    same discard Mortal would have made after declaring reach.
    """

    def __init__(
        self,
        player_state_cls: type,
        default_num_players: int,
        initial_left_tiles: int,
        fast_riichi_left_tiles: int,
        very_fast_riichi_left_tiles: int,
    ) -> None:
        self.player_state_cls = player_state_cls
        self.default_num_players = default_num_players
        self.initial_left_tiles = initial_left_tiles
        self.fast_riichi_left_tiles = fast_riichi_left_tiles
        self.very_fast_riichi_left_tiles = very_fast_riichi_left_tiles
        self.player_id: int | None = None
        self.num_players = default_num_players
        self.left_tiles = initial_left_tiles
        self.rivers: list[list[str]] = [[] for _ in range(max(default_num_players, 4))]
        self.visible_counts = [0] * 34
        self.last_self_tsumo = ""
        self.opponent_hora_count = [0] * max(default_num_players, 4)
        self.opponent_fast_riichi_count = [0] * max(default_num_players, 4)
        self.riichi_seats: set[int] = set()
        self.hot_opponent_seat: int | None = None

    def consume(self, event: dict[str, Any]) -> None:
        t = event.get("type")
        if t == "start_game":
            self.player_id = int(event["id"])
            self.opponent_hora_count = [0] * max(self.default_num_players, 4)
            self.opponent_fast_riichi_count = [0] * max(self.default_num_players, 4)
            self.riichi_seats = set()
            self.hot_opponent_seat = None
            return

        if self.player_id is None:
            return

        if t == "end_game":
            self.player_id = None
            self.hot_opponent_seat = None
            return

        if t == "start_kyoku":
            scores = event.get("scores") or []
            if self.default_num_players == 3:
                self.num_players = 3
            else:
                self.num_players = int(event.get("num_players") or len(scores) or 4)
            self._ensure_vectors()
            self.left_tiles = self.initial_left_tiles
            self.rivers = [[] for _ in range(max(self.num_players, 4))]
            self.visible_counts = [0] * 34
            self.last_self_tsumo = ""
            self.riichi_seats = set()
            for tile in event.get("tehais", [])[self.player_id]:
                _count_tile(self.visible_counts, tile)
            marker = event.get("dora_marker")
            if marker:
                _count_tile(self.visible_counts, marker)
            return

        if t == "dora":
            marker = event.get("dora_marker")
            if marker:
                _count_tile(self.visible_counts, marker)
            return

        if t == "tsumo":
            actor = int(event["actor"])
            self.left_tiles = max(0, self.left_tiles - 1)
            if actor == self.player_id:
                tile = event.get("pai", "")
                _count_tile(self.visible_counts, tile)
                self.last_self_tsumo = tile
            return

        if t == "dahai":
            actor = int(event["actor"])
            tile = event.get("pai", "")
            if actor < len(self.rivers):
                self.rivers[actor].append(tile)
            if actor != self.player_id:
                _count_tile(self.visible_counts, tile)
            else:
                self.last_self_tsumo = ""
            return

        if t == "reach":
            actor = int(event["actor"])
            if actor != self.player_id:
                self._note_opponent_riichi(actor)
            return

        if t == "hora":
            actor = int(event.get("actor", -1))
            self._note_hora(actor)
            return

        if t in ("chi", "pon", "daiminkan"):
            actor = int(event["actor"])
            if actor != self.player_id:
                for tile in event.get("consumed", []):
                    _count_tile(self.visible_counts, tile)
            return

        if t in ("ankan",):
            actor = int(event["actor"])
            if actor != self.player_id:
                for tile in event.get("consumed", []):
                    _count_tile(self.visible_counts, tile)
            return

        if t in ("kakan", "kita", "nukidora"):
            actor = int(event["actor"])
            if actor != self.player_id:
                _count_tile(self.visible_counts, event.get("pai") or "N")

    def suppress_reach_if_needed(
        self,
        action: dict[str, Any],
        event_log: list[str],
        play_style: str,
    ) -> dict[str, Any]:
        if self.player_id is None or action.get("type") != "reach":
            return action
        if int(action.get("actor", self.player_id)) != self.player_id:
            return action

        pai = action.get("pai")
        if not isinstance(pai, str) or not pai:
            return self._suppress_unverified_reach(action, "missing_reach_discard")

        live_waits, waits = self._live_waits_after_dahai(event_log, pai)
        if AUTO_RIICHI_ENABLED and (live_waits is None or live_waits >= MIN_LIVE_WAITS_FOR_RIICHI):
            return action

        reason = "auto_riichi_disabled"
        if live_waits is not None and live_waits < MIN_LIVE_WAITS_FOR_RIICHI:
            reason = "low_live_waits"
        if self.hot_opponent_active():
            reason = HOT_PLAYER_DANGER_NOTE
        elif live_waits is not None and live_waits < MIN_LIVE_WAITS_FOR_RIICHI and play_style == STYLE_CONSERVATIVE:
            reason = "conservative_low_live_waits"

        meta = dict(action.get("meta") or {})
        meta.pop("q_values", None)
        meta.pop("mask_bits", None)
        meta.pop("show", None)
        meta["policy_guard"] = {
            "reach_suppressed": True,
            "reason": reason,
            "live_waits": live_waits,
            "min_live_waits": MIN_LIVE_WAITS_FOR_RIICHI,
            "waits": waits,
            "hot_opponent_seat": self.hot_opponent_seat,
        }
        return {
            "type": "dahai",
            "actor": self.player_id,
            "pai": pai,
            "tsumogiri": _normalize(pai) == _normalize(self.last_self_tsumo),
            "meta": meta,
        }

    def _suppress_unverified_reach(
        self,
        action: dict[str, Any],
        reason: str,
    ) -> dict[str, Any]:
        meta = dict(action.get("meta") or {})
        pai = _best_discard_from_meta(meta)
        fallback_reason = reason
        if not pai and self.last_self_tsumo:
            pai = self.last_self_tsumo
            fallback_reason = f"{reason}_tsumogiri_fallback"

        meta.pop("q_values", None)
        meta.pop("mask_bits", None)
        meta.pop("show", None)
        meta["policy_guard"] = {
            "reach_suppressed": True,
            "reason": fallback_reason,
            "min_live_waits": MIN_LIVE_WAITS_FOR_RIICHI,
            "hot_opponent_seat": self.hot_opponent_seat,
        }
        if pai:
            meta["policy_guard"]["fallback_dahai"] = pai
            return {
                "type": "dahai",
                "actor": self.player_id,
                "pai": pai,
                "tsumogiri": _normalize(pai) == _normalize(self.last_self_tsumo),
                "meta": meta,
            }
        return {
            "type": "none",
            "actor": self.player_id,
            "meta": meta,
        }

    def guard_action(
        self,
        action: dict[str, Any],
        event_log: list[str],
        play_style: str,
    ) -> dict[str, Any]:
        action = self.suppress_reach_if_needed(action, event_log, play_style)
        return self._defensive_dahai_if_needed(action, play_style)

    def hot_opponent_active(self) -> bool:
        return self.hot_opponent_seat is not None and self.hot_opponent_seat != self.player_id

    def _defence_mode(self, play_style: str) -> bool:
        return play_style == STYLE_CONSERVATIVE or self.hot_opponent_active()

    def _late_defence_active(self, play_style: str) -> bool:
        if play_style != STYLE_CONSERVATIVE:
            return False
        threshold = (
            THREE_PLAYER_LATE_DEFENCE_LEFT_TILES
            if self.num_players == 3
            else FOUR_PLAYER_LATE_DEFENCE_LEFT_TILES
        )
        return self.left_tiles <= threshold

    def _defensive_dahai_if_needed(
        self,
        action: dict[str, Any],
        play_style: str,
    ) -> dict[str, Any]:
        if self.player_id is None or action.get("type") != "dahai":
            return action
        if int(action.get("actor", self.player_id)) != self.player_id:
            return action

        active_riichi = sorted(
            seat for seat in self.riichi_seats if seat != self.player_id and seat < self.num_players
        )
        late_defence = self._late_defence_active(play_style)
        hot_defence = self.hot_opponent_active()
        if not active_riichi and not late_defence and not hot_defence:
            return action

        danger_seats = set(active_riichi)
        if self.hot_opponent_seat is not None and self.hot_opponent_seat != self.player_id:
            danger_seats.add(self.hot_opponent_seat)
        if late_defence:
            danger_seats.update(
                seat
                for seat in range(self.num_players)
                if seat != self.player_id
            )
        if not danger_seats:
            return action

        meta = dict(action.get("meta") or {})
        candidates = _discard_candidates_from_meta(meta)
        current_pai = action.get("pai")
        if isinstance(current_pai, str) and current_pai:
            candidates.append((current_pai, float("-inf")))
        if not candidates:
            return action

        best_pai, _ = max(
            candidates,
            key=lambda item: self._defensive_discard_score(item[0], item[1], danger_seats),
        )
        if not isinstance(current_pai, str) or best_pai == current_pai:
            return action

        reason = "riichi_defense" if active_riichi else "late_game_conservative"
        if hot_defence:
            reason = HOT_PLAYER_DANGER_NOTE

        meta.pop("q_values", None)
        meta.pop("mask_bits", None)
        meta.pop("show", None)
        meta["policy_guard"] = {
            "defensive_dahai": True,
            "reason": reason,
            "original_pai": current_pai,
            "selected_pai": best_pai,
            "left_tiles": self.left_tiles,
            "danger_seats": sorted(danger_seats),
            "riichi_seats": active_riichi,
            "hot_opponent_seat": self.hot_opponent_seat,
        }
        return {
            "type": "dahai",
            "actor": self.player_id,
            "pai": best_pai,
            "tsumogiri": _normalize(best_pai) == _normalize(self.last_self_tsumo),
            "meta": meta,
        }

    def _defensive_discard_score(
        self,
        tile: str,
        model_score: float,
        danger_seats: set[int],
    ) -> tuple[int, int, int, int, int, int, float]:
        norm = _normalize(tile)
        genbutsu_count = 0
        for seat in danger_seats:
            if seat < len(self.rivers) and any(_normalize(discard) == norm for discard in self.rivers[seat]):
                genbutsu_count += 1

        idx = _to34(tile)
        visible = self.visible_counts[idx] if 0 <= idx < len(self.visible_counts) else 0
        honor = 1 if 27 <= idx < 34 else 0
        terminal = 1 if 0 <= idx < 27 and idx % 9 in (0, 8) else 0
        non_red = 1 if "r" not in tile else 0
        return (
            genbutsu_count,
            1 if visible >= 4 else 0,
            visible,
            honor,
            terminal,
            non_red,
            model_score,
        )

    def _live_waits_after_dahai(
        self,
        event_log: list[str],
        pai: str,
    ) -> tuple[int | None, list[int]]:
        state = self.player_state_cls(self.player_id)
        for raw in event_log:
            state.update(raw)
        state.update(
            json.dumps(
                {
                    "type": "dahai",
                    "actor": self.player_id,
                    "pai": pai,
                    "tsumogiri": _normalize(pai) == _normalize(self.last_self_tsumo),
                },
                separators=(",", ":"),
            )
        )
        waits = [idx for idx, enabled in enumerate(state.waits) if enabled]
        visible = list(self.visible_counts)
        live_waits = 0
        for idx in waits:
            if idx == _to34(pai):
                visible[idx] = max(visible[idx], 1)
            live_waits += max(0, 4 - visible[idx])
        return live_waits, waits

    def _ensure_vectors(self) -> None:
        size = max(self.num_players, 4)
        if len(self.opponent_hora_count) < size:
            self.opponent_hora_count.extend([0] * (size - len(self.opponent_hora_count)))
        if len(self.opponent_fast_riichi_count) < size:
            self.opponent_fast_riichi_count.extend(
                [0] * (size - len(self.opponent_fast_riichi_count))
            )

    def _note_opponent_riichi(self, actor: int) -> None:
        self._ensure_vectors()
        if actor == self.player_id or not (0 <= actor < len(self.opponent_fast_riichi_count)):
            return
        self.riichi_seats.add(actor)
        river_len = len(self.rivers[actor]) if actor < len(self.rivers) else 99
        if self.left_tiles >= self.very_fast_riichi_left_tiles or river_len <= 3:
            self.hot_opponent_seat = actor
            return
        if self.left_tiles >= self.fast_riichi_left_tiles or river_len <= 5:
            self.opponent_fast_riichi_count[actor] += 1
            if self.opponent_fast_riichi_count[actor] >= HOT_PLAYER_FAST_RIICHI_COUNT:
                self.hot_opponent_seat = actor

    def _note_hora(self, actor: int) -> None:
        self._ensure_vectors()
        if actor == self.player_id or not (0 <= actor < len(self.opponent_hora_count)):
            return
        self.opponent_hora_count[actor] += 1
        if self.opponent_hora_count[actor] >= HOT_PLAYER_WIN_COUNT:
            self.hot_opponent_seat = actor
