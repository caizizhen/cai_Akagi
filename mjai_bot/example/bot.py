# Akagi v3 example bot: rule-based shanten optimizer.
#
# Algorithm summary:
#   AwaitingDiscard (own tsumo):
#     1. Tsumo agari if reachable.
#     2. Ankan / Kakan if shanten doesn't worsen — skipped while defending
#        (riichi or live score pressure) to avoid flipping dora.
#     3. Riichi when tenpai with >= 3 waits — stricter if others are riichi,
#        thin wall or score pressure;
#        pick riichi maximising wait count.
#     4. Discard: minimise shanten; under defence (riichi or score pressure)
#        tiebreak by danger then ukeire; may fold +1 shanten if danger drops enough.
#   AwaitingResponse (others' dahai / kakan):
#     1. Ron if reachable.
#     2. Daiminkan / Pon / Chi when shanten strictly decreases AND a yaku
#        path exists — early wall OR score pressure: extra conservatism.
#        Chi / Pon pick best shape when multiple consumptions are legal (incl. red 5).
#        Daiminkan > Pon > Chi by precedence.
#     3. Otherwise pass.
#   Sanma (num_players=3):
#     - Consume `kita` (北抜き) for all seats; optional `kita` reply after self tsumo
#       when it does not worsen shanten.

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass, field
from pathlib import Path
from mahjong.constants import EAST, NORTH, SOUTH, WEST
from mahjong.hand_calculating.hand import HandCalculator
from mahjong.hand_calculating.hand_config import HandConfig
from mahjong.meld import Meld
from mahjong.shanten import Shanten

# ---------- mjai tile string ↔ 34-array index ----------

# 0-8 m, 9-17 p, 18-26 s, 27 E, 28 S, 29 W, 30 N, 31 P, 32 F, 33 C
_HONOR = {"E": 27, "S": 28, "W": 29, "N": 30, "P": 31, "F": 32, "C": 33}
_HONOR_REV = {v: k for k, v in _HONOR.items()}
_WIND_MJAI = {EAST: "E", SOUTH: "S", WEST: "W", NORTH: "N"}


def _normalize(tile: str) -> str:
    """Drop akadora 'r' suffix for shanten/identity purposes."""
    return tile[:-1] if tile.endswith("r") else tile


def _to34(tile: str) -> int:
    t = _normalize(tile)
    if t in _HONOR:
        return _HONOR[t]
    n = int(t[0])
    if n == 0:
        n = 5
    suit = t[1]
    return {"m": 0, "p": 9, "s": 18}[suit] + (n - 1)


def _from34(idx: int) -> str:
    if idx >= 27:
        return _HONOR_REV[idx]
    suit = "mps"[idx // 9]
    return f"{(idx % 9) + 1}{suit}"


def _is_yaochuu(idx: int) -> bool:
    if idx >= 27:
        return True
    n = idx % 9
    return n == 0 or n == 8


# ---------- defence heuristics (Scheme A) ----------
# Defaults; override via Bots UI → settings.toml → AKAGI_BOT_CONFIG JSON.

_DEF_EARLY_LEFT_TILES: int = 56
_DEF_MIN_LEFT_FOR_RIICHI: int = 10
_DEF_TIGHT_FIRST_PLACE_MARGIN: int = 7200
_DEF_FOLD_TRIGGER_DANGER: int = 35
_DEF_FOLD_MIN_DANGER_DROP: int = 10
_DEF_RIICHI_MAX_LEFT_2_WAIT_RIVALS: int = 16
_DEF_RIICHI_MAX_LEFT_2_WAIT_PRESSURE: int = 18
_DEF_RIICHI_MIN_SCORE: int = 34
_DEF_CALL_VALUE_PENALTY_MAX: int = 24
_DEF_MIN_SAFE_TILES_TO_FOLD: int = 2
_DEF_DAMATEN_VALUE_SCORE: int = 50
_STYLE_AGGRESSIVE = "aggressive"
_STYLE_BALANCED = "balanced"
_STYLE_CONSERVATIVE = "conservative"
_STYLE_TANYAO_FAST = "tanyao_fast"
_EXPERIENCE_VERSION: int = 1


def _experience_path() -> Path:
    p = os.environ.get("AKAGI_EXPERIENCE_PATH")
    if p:
        return Path(p)
    return Path(__file__).with_name("experience.json")


def _default_experience() -> dict:
    return {
        "version": _EXPERIENCE_VERSION,
        "games": 0,
        "totals": {
            "kyoku": 0,
            "decisions": 0,
            "riichi": 0,
            "calls": 0,
            "calls_vs_riichi": 0,
            "dahai": 0,
            "dangerous_dahai": 0,
            "hora": 0,
            "deal_in": 0,
            "ryukyoku": 0,
        },
        "recent": [],
    }


def _load_experience() -> dict:
    path = _experience_path()
    try:
        with path.open("r", encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, json.JSONDecodeError):
        return _default_experience()
    if not isinstance(data, dict):
        return _default_experience()
    base = _default_experience()
    base.update(data)
    totals = base.get("totals")
    if not isinstance(totals, dict):
        base["totals"] = _default_experience()["totals"]
    else:
        for k, v in _default_experience()["totals"].items():
            totals.setdefault(k, v)
    if not isinstance(base.get("recent"), list):
        base["recent"] = []
    return base


def _save_experience(data: dict) -> None:
    path = _experience_path()
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(path.suffix + ".tmp")
        with tmp.open("w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2, sort_keys=True)
            f.write("\n")
        tmp.replace(path)
    except OSError:
        pass


def _new_game_stats() -> dict:
    return {
        "num_players": 4,
        "kyoku": 0,
        "decisions": 0,
        "riichi": 0,
        "calls": 0,
        "calls_vs_riichi": 0,
        "dahai": 0,
        "dangerous_dahai": 0,
        "hora": 0,
        "deal_in": 0,
        "ryukyoku": 0,
        "start_score": None,
        "end_score": None,
        "score_delta": 0,
        "notes": [],
    }


def _cfg_int(cfg: dict, key: str, default: int) -> int:
    v = cfg.get(key)
    if v is None:
        return default
    try:
        return int(v)
    except (TypeError, ValueError):
        return default


def _cfg_str(cfg: dict, key: str, default: str) -> str:
    v = cfg.get(key)
    return v if isinstance(v, str) and v else default


def _fresh_bot_config() -> dict:
    """Resolved settings from env (see Akagi `AKAGI_BOT_CONFIG`)."""
    p = os.environ.get("AKAGI_BOT_CONFIG")
    if not p:
        return {}
    try:
        with open(p, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return {}


def _avoid_kan_for_defense(state: State) -> bool:
    """Ankan/kakan flip dora; skip while defending."""
    if _play_style(state) == _STYLE_AGGRESSIVE:
        return False
    return bool(state.rivals_riichi) or _score_table_pressure(state)


def _next_dora_index(indicator_idx: int) -> int:
    """Dora tile index in 34-form given a dora-indicator index."""
    if indicator_idx >= 27:
        return 27 + ((indicator_idx - 27 + 1) % 7)
    suit = indicator_idx // 9
    r = indicator_idx % 9
    return suit * 9 + (r + 1) % 9


def _dora_indices(state: State) -> set[int]:
    return {_next_dora_index(_to34(t)) for t in state.dora_indicators}


def _seat_is_oya(seat: int, state: State) -> bool:
    return seat == state.oya


def _is_genbutsu_vs_seat(tile_idx: int, seat: int, state: State) -> bool:
    """Tile is among this seat's discards ⇒ cannot ron our discard from them."""
    return any(_to34(t) == tile_idx for t in state.rivers[seat])


def _suji_relief(tile_idx: int, seat: int, state: State) -> int:
    """Rough suji bonus (subtracted from danger) from this seat's river."""
    if tile_idx >= 27:
        return 0
    suit = tile_idx // 9
    rank = tile_idx % 9 + 1
    best = 0
    for t in state.rivers[seat]:
        ti = _to34(t)
        if ti >= 27 or ti // 9 != suit:
            continue
        r = ti % 9 + 1
        for dr in (-3, 3):
            nr = r + dr
            if 1 <= nr <= 9 and nr == rank:
                best = max(best, 20)
    return best


def _wall_relief(tile_idx: int, state: State) -> int:
    """Simple kabe / outside-tile relief from visible adjacent ranks."""
    if tile_idx >= 27:
        return 0
    suit = tile_idx // 9
    rank = tile_idx % 9 + 1
    relief = 0
    for near in (rank - 1, rank + 1):
        if 1 <= near <= 9:
            vis = state.visible_count(suit * 9 + (near - 1))
            if vis >= 4:
                relief = max(relief, 18)
            elif vis >= 3:
                relief = max(relief, 10)
    if rank in (1, 9):
        near = 2 if rank == 1 else 8
        if state.visible_count(suit * 9 + (near - 1)) >= 3:
            relief = max(relief, 14)
    return relief


def _danger_for_seat(tile_idx: int, seat: int, state: State, *, hard: bool) -> int:
    """Deal-in danger vs `seat`. `hard=True` (riichi); softer weights if False."""
    if _is_genbutsu_vs_seat(tile_idx, seat, state):
        return 0
    remaining = 4 - state.visible_count(tile_idx)
    if remaining <= 0:
        return 0
    yakuhai = _is_yakuhai_idx(state, tile_idx)
    if tile_idx >= 27:
        base = 34 if hard else 22
        if yakuhai:
            base += 10 if hard else 7
    else:
        base = 48 if hard else 28
    d = base - _suji_relief(tile_idx, seat, state) - _wall_relief(tile_idx, state)
    if hard and _seat_is_oya(seat, state):
        d += 10
    dora_bonus = 24 if hard else 16
    if tile_idx in _dora_indices(state):
        d += dora_bonus
    vis = state.visible_count(tile_idx)
    if tile_idx >= 27 and vis >= 2:
        d -= 18 if hard else 12
    if vis >= 3:
        d -= 14 if hard else 10
    elif vis >= 2:
        d -= 6 if hard else 4
    return max(0, d)


def _rivals_big_win_potential(state: State) -> bool:
    """Observable hints that opponents may still collect mangan-class value."""
    if state.rivals_riichi:
        return True
    if len(state.dora_indicators) >= 2:
        return True
    for s in range(state.num_players):
        if s == state.actor_id:
            continue
        melds = state.table_melds[s]
        if len(melds) >= 2:
            return True
        for m in melds:
            if m["type"] == "daiminkan":
                return True
            if m["type"] in ("pon", "kakan") and _is_yakuhai_idx(state, _to34(m["tiles"][0])):
                return True
    if (
        state.scores
        and len(state.scores) >= state.num_players
        and state.num_players >= 2
    ):
        mine = state.scores[state.actor_id]
        others = [state.scores[i] for i in range(state.num_players) if i != state.actor_id]
        if others:
            mx = max(others)
            # Thin lead as 1st: one big hand reshuffles placement.
            if mine >= mx and (mine - mx) < _tight_first_place_margin(state):
                return True
    return False


def _is_late_round(state: State) -> bool:
    if state.bakaze == "S":
        return True
    return state.kyoku >= state.num_players


def _placement_pressure(state: State) -> bool:
    """Current score/round pressure where avoiding a swing matters more."""
    if not state.scores or len(state.scores) < state.num_players:
        return False
    mine = state.scores[state.actor_id]
    others = [state.scores[i] for i in range(state.num_players) if i != state.actor_id]
    if not others:
        return False
    ahead = [s for s in others if s > mine]
    best_other = max(others)
    if not ahead:
        return mine - best_other < _tight_first_place_margin(state)
    if _is_late_round(state):
        return min(ahead) - mine <= _tight_first_place_margin(state)
    return False


def _score_table_pressure(state: State) -> bool:
    """Live placement pressure plus observable opponent value hints."""
    return _placement_pressure(state) and _rivals_big_win_potential(state)


def _discard_danger(tile_idx: int, state: State) -> int:
    """Worst-case discard danger vs riichi seats (hard) and/or all rivals (soft)."""
    by_seat: dict[int, bool] = {}
    for s in state.rivals_riichi:
        by_seat[s] = True
    if _score_table_pressure(state):
        for s in range(state.num_players):
            if s != state.actor_id:
                by_seat.setdefault(s, False)
    if not by_seat:
        return 0
    return max(_danger_for_seat(tile_idx, s, state, hard=h) for s, h in by_seat.items())


def _safety_inventory(state: State, hand34: list[int] | None = None) -> dict:
    """Count safe-ish discard options currently in hand under live pressure."""
    hand = hand34 if hand34 is not None else state.hand34
    dangers = [_discard_danger(idx, state) for idx, n in enumerate(hand) if n > 0]
    if not dangers:
        return {
            "safe": 0,
            "low": 0,
            "usable": 0,
            "best_danger": 0,
            "worst_danger": 0,
        }
    trigger = _fold_trigger_danger(state)
    return {
        "safe": sum(1 for d in dangers if d == 0),
        "low": sum(1 for d in dangers if d < 20),
        "usable": sum(1 for d in dangers if d < trigger),
        "best_danger": min(dangers),
        "worst_danger": max(dangers),
    }


def _is_yakuhai_idx(state: State, idx: int) -> bool:
    sw = seat_wind_const(state.actor_id, state.oya) - EAST + 27
    rw = _to34(state.bakaze)
    return idx >= 31 or idx == sw or idx == rw


def _allow_open_call(
    state: State,
    meld_type: str,
    called_idx: int,
    current_sh: int,
    best_sh: int,
    best_discard_danger: int | None = None,
) -> bool:
    """Tighter melds in early wall; yakuhai pon still welcome."""
    style = _play_style(state)
    if style == _STYLE_TANYAO_FAST:
        if called_idx >= 27 or _is_yaochuu(called_idx):
            return False
        return best_sh <= current_sh
    if best_sh >= current_sh:
        return False
    if style == _STYLE_AGGRESSIVE:
        return True
    if style == _STYLE_CONSERVATIVE:
        if meld_type == "daiminkan":
            return False
        if bool(state.rivals_riichi) or _score_table_pressure(state):
            return best_sh == 0 and best_discard_danger is not None and best_discard_danger < _fold_trigger_danger(state)
        return best_sh <= 1 and _is_yakuhai_idx(state, called_idx)
    if state.rivals_riichi:
        if meld_type == "daiminkan":
            return False
        if best_sh != 0:
            return False
        if best_discard_danger is None:
            return False
        return best_discard_danger < _fold_trigger_danger(state)
    if _score_table_pressure(state):
        yakuhai = _is_yakuhai_idx(state, called_idx)
        if meld_type == "pon" and yakuhai:
            return best_sh < current_sh
        return best_sh == 0 or best_sh <= current_sh - 2
    if state.left_tiles < _early_left_tiles(state):
        return True
    yakuhai = _is_yakuhai_idx(state, called_idx)
    if meld_type == "pon" and yakuhai:
        return True
    if meld_type == "chi":
        return best_sh == 0 or best_sh <= current_sh - 2
    if meld_type == "pon":
        return best_sh == 0 or best_sh <= current_sh - 2
    if meld_type == "daiminkan":
        return best_sh == 0 or best_sh <= current_sh - 2
    return True


# ---------- 34 → 136 conversion ----------

def _hand34_to_136(counts34: list[int], aka_in_hand: dict[str, int]) -> list[int]:
    """Concealed-hand 34-array → 136-array. Akadora occupy slot 16,
    52, 88 (the first index of 5m, 5p, 5s blocks)."""
    used: dict[int, int] = {}
    out: list[int] = []
    aka_remaining = dict(aka_in_hand)
    for idx, n in enumerate(counts34):
        for _ in range(n):
            base = idx * 4
            slot = used.get(idx, 0)
            tile136 = base + slot
            # Promote slot 0 to akadora form if this is a red 5 owed.
            if idx in (4, 13, 22):  # 5m, 5p, 5s
                suit = "mps"[idx // 9]
                if aka_remaining.get(suit, 0) > 0 and slot == 0:
                    aka_remaining[suit] -= 1
            out.append(tile136)
            used[idx] = slot + 1
    return out


def _tile136_for_index(idx: int) -> int:
    return idx * 4


# ---------- meld → mahjong.Meld ----------

def _meld_to_mahjong(meld: dict) -> Meld:
    """Translate our internal meld dict into a mahjong.Meld. Tile
    indices are coarse 4*idx+0; mahjong only inspects tiles_34 for most
    yaku checks so the exact 136 slot doesn't matter here."""
    tiles136 = [_tile136_for_index(_to34(t)) for t in meld["tiles"]]
    if meld["type"] == "chi":
        return Meld(meld_type=Meld.CHI, tiles=tiles136, opened=True)
    if meld["type"] == "pon":
        return Meld(meld_type=Meld.PON, tiles=tiles136, opened=True)
    if meld["type"] == "daiminkan":
        return Meld(meld_type=Meld.KAN, tiles=tiles136, opened=True)
    if meld["type"] == "kakan":
        return Meld(meld_type=Meld.SHOUMINKAN, tiles=tiles136, opened=True)
    if meld["type"] == "ankan":
        return Meld(meld_type=Meld.KAN, tiles=tiles136, opened=False)
    raise ValueError(f"unknown meld type: {meld['type']}")


# ---------- state ----------

@dataclass
class State:
    actor_id: int = 0
    oya: int = 0
    bakaze: str = "E"
    kyoku: int = 1
    honba: int = 0
    kyotaku: int = 0
    # Concealed hand only (melds tracked separately).
    hand34: list[int] = field(default_factory=lambda: [0] * 34)
    aka_in_hand: dict[str, int] = field(default_factory=lambda: {"m": 0, "p": 0, "s": 0})
    melds: list[dict] = field(default_factory=list)
    rivers: list[list[str]] = field(default_factory=lambda: [[] for _ in range(4)])
    river_called: list[list[bool]] = field(default_factory=lambda: [[] for _ in range(4)])
    last_self_tsumo: str = ""
    reach_declared: bool = False
    reach_accepted: bool = False
    left_tiles: int = 70
    dora_indicators: list[str] = field(default_factory=list)
    # Seats (other than self) with accepted riichi this kyoku.
    rivals_riichi: set[int] = field(default_factory=set)
    num_players: int = 4
    scores: list[int] = field(default_factory=list)
    # Open melds per seat (visible to all); mirrors self.melds for our seat.
    table_melds: list[list[dict]] = field(default_factory=lambda: [[] for _ in range(4)])
    # Sanma: count of kita (北抜き) per seat — each removes one visible North (34-index 30).
    kita_count: list[int] = field(default_factory=lambda: [0, 0, 0, 0])
    # Reloaded on each `start_game` from AKAGI_BOT_CONFIG (Bots settings).
    bot_cfg: dict = field(default_factory=dict)
    experience: dict = field(default_factory=dict)
    game_stats: dict = field(default_factory=_new_game_stats)
    experience_saved: bool = False
    # ----- mutators -----

    def consume(self, e: dict) -> None:
        t = e.get("type")
        if t == "start_game":
            if "id" in e:
                self.actor_id = int(e["id"])
            self.bot_cfg = _fresh_bot_config()
            self.experience = _load_experience()
            self.game_stats = _new_game_stats()
            self.rivals_riichi = set()
            self.scores = []
            self.kita_count = [0, 0, 0, 0]
            self.experience_saved = False
        elif t == "start_kyoku":
            self.oya = e["oya"]
            self.bakaze = e["bakaze"]
            self.kyoku = e["kyoku"]
            self.honba = e["honba"]
            self.kyotaku = e["kyotaku"]
            self.num_players = int(e.get("num_players", 4))
            self.scores = list(e["scores"])
            self.game_stats["num_players"] = self.num_players
            self.game_stats["kyoku"] += 1
            if len(self.scores) > self.actor_id and self.game_stats["start_score"] is None:
                self.game_stats["start_score"] = self.scores[self.actor_id]
            self.hand34 = [0] * 34
            self.aka_in_hand = {"m": 0, "p": 0, "s": 0}
            for tile in e["tehais"][self.actor_id]:
                self._add(tile)
            self.melds = []
            self.table_melds = [[] for _ in range(4)]
            self.rivers = [[] for _ in range(4)]
            self.river_called = [[] for _ in range(4)]
            self.last_self_tsumo = ""
            self.reach_declared = False
            self.reach_accepted = False
            self.left_tiles = 70
            self.dora_indicators = [e["dora_marker"]]
            self.rivals_riichi = set()
            self.kita_count = [0, 0, 0, 0]
        elif t == "kita":
            a = int(e["actor"])
            pai = e.get("pai") or "N"
            if not pai:
                pai = "N"
            self.kita_count[a] += 1
            if a == self.actor_id:
                self._remove(pai)
        elif t == "reach":
            actor = int(e["actor"])
            if actor == self.actor_id:
                self.reach_declared = True
        elif t == "dora":
            self.dora_indicators.append(e["dora_marker"])
        elif t == "tsumo":
            if e["actor"] == self.actor_id:
                self._add(e["pai"])
                self.last_self_tsumo = e["pai"]
            self.left_tiles = max(0, self.left_tiles - 1)
        elif t == "dahai":
            actor = e["actor"]
            self.rivers[actor].append(e["pai"])
            self.river_called[actor].append(False)
            if actor == self.actor_id:
                self._remove(e["pai"])
                self.last_self_tsumo = ""
        elif t == "reach_accepted":
            a = e["actor"]
            if a == self.actor_id:
                self.reach_accepted = True
            else:
                self.rivals_riichi.add(a)
        elif t == "hora":
            d = e.get("deltas")
            if d and self.scores and len(d) == len(self.scores):
                self.scores = [a + b for a, b in zip(self.scores, d)]
        elif t == "ryukyoku":
            d = e.get("deltas")
            if d and self.scores and len(d) == len(self.scores):
                self.scores = [a + b for a, b in zip(self.scores, d)]
        elif t in ("chi", "pon", "daiminkan"):
            target = e["target"]
            if self.river_called[target]:
                self.river_called[target][-1] = True
            act = e["actor"]
            meld_rec = {
                "type": t,
                "tiles": list(e["consumed"]) + [e["pai"]],
                "from_seat": target,
                "called_tile": e["pai"],
            }
            self.table_melds[act].append(meld_rec)
            if act == self.actor_id:
                for c in e["consumed"]:
                    self._remove(c)
                self.melds.append(meld_rec)
        elif t == "kakan":
            act = e["actor"]
            if act == self.actor_id:
                self._remove(e["pai"])
                for m in self.melds:
                    if m["type"] == "pon" and _normalize(m["tiles"][0]) == _normalize(e["pai"]):
                        m["type"] = "kakan"
                        m["tiles"].append(e["pai"])
                        break
            for m in self.table_melds[act]:
                if m["type"] == "pon" and _normalize(m["tiles"][0]) == _normalize(e["pai"]):
                    m["type"] = "kakan"
                    m["tiles"].append(e["pai"])
                    break
        elif t == "ankan":
            act = e["actor"]
            rec = {
                "type": "ankan",
                "tiles": list(e["consumed"]),
                "from_seat": act,
            }
            self.table_melds[act].append(rec)
            if act == self.actor_id:
                for c in e["consumed"]:
                    self._remove(c)
                self.melds.append(rec)

    def _add(self, tile: str) -> None:
        if tile == "?":
            # Unknown tile (other seats' hidden hand). Should never reach
            # us for our own seat; skip defensively.
            return
        self.hand34[_to34(tile)] += 1
        if tile.endswith("r"):
            self.aka_in_hand[_normalize(tile)[1]] += 1

    def _remove(self, tile: str) -> None:
        if tile == "?":
            return
        idx = _to34(tile)
        if self.hand34[idx] > 0:
            self.hand34[idx] -= 1
        if tile.endswith("r") and self.aka_in_hand[_normalize(tile)[1]] > 0:
            self.aka_in_hand[_normalize(tile)[1]] -= 1

    # ----- queries -----

    def total_tiles(self) -> int:
        return sum(self.hand34) + sum(len(m["tiles"]) - (1 if m["type"] in ("ankan", "daiminkan", "kakan") else 0) for m in self.melds)

    def is_closed(self) -> bool:
        return all(m["type"] == "ankan" for m in self.melds)

    def seat_wind_mjai(self) -> str:
        return _WIND_MJAI[seat_wind_const(self.actor_id, self.oya)]

    def round_wind_mjai(self) -> str:
        return self.bakaze

    def kamicha(self) -> int:
        return (self.actor_id + self.num_players - 1) % self.num_players

    def visible_count(self, idx: int) -> int:
        n = self.hand34[idx]
        for seat in range(4):
            for m in self.table_melds[seat]:
                for t in m["tiles"]:
                    if _to34(t) == idx:
                        n += 1
        for s in range(4):
            for t in self.rivers[s]:
                if _to34(t) == idx:
                    n += 1
        for ind in self.dora_indicators:
            if _to34(ind) == idx:
                n += 1
        if idx == 30:
            n += sum(self.kita_count)
        return n


def _record_experience_events(state: State, events: list[dict]) -> None:
    stats = state.game_stats
    for ev in events:
        t = ev.get("type")
        if t == "hora":
            actor = ev.get("actor")
            target = ev.get("target")
            if actor == state.actor_id:
                stats["hora"] += 1
                stats["notes"].append("won hand")
            if target == state.actor_id and actor != state.actor_id:
                stats["deal_in"] += 1
                stats["notes"].append("dealt in")
        elif t == "ryukyoku":
            stats["ryukyoku"] += 1
        elif t == "end_game":
            if state.scores and len(state.scores) > state.actor_id:
                stats["end_score"] = state.scores[state.actor_id]
                if stats["start_score"] is not None:
                    stats["score_delta"] = stats["end_score"] - stats["start_score"]


def _record_experience_action(state: State, action: dict) -> None:
    t = action.get("type")
    if not t or t == "none":
        return
    stats = state.game_stats
    stats["decisions"] += 1
    if t == "reach":
        stats["riichi"] += 1
    elif t in ("chi", "pon", "daiminkan"):
        stats["calls"] += 1
        if state.rivals_riichi:
            stats["calls_vs_riichi"] += 1
    elif t == "dahai":
        stats["dahai"] += 1
        pai = action.get("pai")
        if pai:
            danger = _discard_danger(_to34(pai), state)
            if danger >= _fold_trigger_danger(state):
                stats["dangerous_dahai"] += 1


def _experience_summary(stats: dict) -> dict:
    summary = {k: stats.get(k, 0) for k in (
        "num_players",
        "kyoku",
        "decisions",
        "riichi",
        "calls",
        "calls_vs_riichi",
        "dahai",
        "dangerous_dahai",
        "hora",
        "deal_in",
        "ryukyoku",
        "score_delta",
    )}
    notes = []
    if stats.get("deal_in", 0) > 0:
        notes.append("review deal-in turns")
    if stats.get("dangerous_dahai", 0) > 0:
        notes.append("dangerous discards under pressure")
    if stats.get("calls_vs_riichi", 0) > 0:
        notes.append("called after rival riichi")
    if stats.get("riichi", 0) == 0 and stats.get("hora", 0) == 0:
        notes.append("low reach/win activity")
    summary["notes"] = notes[:4]
    return summary


def _finalize_experience(state: State) -> None:
    if state.experience_saved:
        return
    exp = state.experience or _default_experience()
    stats = state.game_stats
    exp["games"] = int(exp.get("games", 0)) + 1
    totals = exp.setdefault("totals", {})
    for k in _default_experience()["totals"]:
        totals[k] = int(totals.get(k, 0)) + int(stats.get(k, 0))
    recent = exp.setdefault("recent", [])
    recent.append(_experience_summary(stats))
    del recent[:-20]
    exp["last_summary"] = recent[-1]
    state.experience = exp
    state.experience_saved = True
    _save_experience(exp)


def _early_left_tiles(state: State) -> int:
    return _cfg_int(state.bot_cfg, "early_left_tiles", _DEF_EARLY_LEFT_TILES)


def _play_style(state: State) -> str:
    style = _cfg_str(state.bot_cfg, "play_style", _STYLE_BALANCED)
    if style in {_STYLE_AGGRESSIVE, _STYLE_BALANCED, _STYLE_CONSERVATIVE, _STYLE_TANYAO_FAST}:
        return style
    return _STYLE_BALANCED


def _min_left_for_riichi(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "min_left_for_riichi", _DEF_MIN_LEFT_FOR_RIICHI)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return max(2, base - 6)
    if style == _STYLE_TANYAO_FAST:
        return max(2, base - 4)
    if style == _STYLE_CONSERVATIVE:
        return min(30, base + 6)
    return base


def _tight_first_place_margin(state: State) -> int:
    return _cfg_int(state.bot_cfg, "tight_first_place_margin", _DEF_TIGHT_FIRST_PLACE_MARGIN)


def _fold_trigger_danger(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "fold_trigger_danger", _DEF_FOLD_TRIGGER_DANGER)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return 90
    if style == _STYLE_TANYAO_FAST:
        return 80
    if style == _STYLE_CONSERVATIVE:
        return max(8, min(base, 18))
    return base


def _fold_min_danger_drop(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "fold_min_danger_drop", _DEF_FOLD_MIN_DANGER_DROP)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return 40
    if style == _STYLE_TANYAO_FAST:
        return 35
    if style == _STYLE_CONSERVATIVE:
        return max(0, min(base, 4))
    return base


def _riichi_max_left_2_wait_rivals(state: State) -> int:
    return _cfg_int(
        state.bot_cfg,
        "riichi_max_left_2_wait_rivals",
        _DEF_RIICHI_MAX_LEFT_2_WAIT_RIVALS,
    )


def _riichi_max_left_2_wait_pressure(state: State) -> int:
    return _cfg_int(
        state.bot_cfg,
        "riichi_max_left_2_wait_pressure",
        _DEF_RIICHI_MAX_LEFT_2_WAIT_PRESSURE,
    )


def _riichi_min_score(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "riichi_min_score", _DEF_RIICHI_MIN_SCORE)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return max(0, base - 16)
    if style == _STYLE_TANYAO_FAST:
        return 120
    if style == _STYLE_BALANCED:
        return min(120, base + 28)
    if style == _STYLE_CONSERVATIVE:
        return min(120, base + 24)
    return base


def _call_value_penalty_max(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "call_value_penalty_max", _DEF_CALL_VALUE_PENALTY_MAX)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return max(base, 80)
    if style == _STYLE_TANYAO_FAST:
        return max(base, 64)
    if style == _STYLE_CONSERVATIVE:
        return min(base, 12)
    return base


def _min_safe_tiles_to_fold(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "min_safe_tiles_to_fold", _DEF_MIN_SAFE_TILES_TO_FOLD)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return 0
    if style == _STYLE_TANYAO_FAST:
        return 0
    if style == _STYLE_CONSERVATIVE:
        return max(base, 5)
    return base


def _damaten_value_score(state: State) -> int:
    base = _cfg_int(state.bot_cfg, "damaten_value_score", _DEF_DAMATEN_VALUE_SCORE)
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return 120
    if style == _STYLE_TANYAO_FAST:
        return 120
    if style == _STYLE_BALANCED:
        return min(base, 34)
    if style == _STYLE_CONSERVATIVE:
        return min(base, 36)
    return base


def _debug_meta(state: State, reason: str, **kwargs) -> dict:
    data = {
        "reason": reason,
        "left_tiles": state.left_tiles,
        "rivals_riichi": sorted(state.rivals_riichi),
        "score_pressure": _score_table_pressure(state),
        "play_style": _play_style(state),
    }
    data.update(kwargs)
    return {"debug": data}


def _with_meta(action: dict, state: State, reason: str, **kwargs) -> dict:
    if action.get("type") != "none":
        action = dict(action)
        action["meta"] = _debug_meta(state, reason, **kwargs)
    return action


def seat_wind_const(seat: int, oya: int) -> int:
    return [EAST, SOUTH, WEST, NORTH][(seat - oya) % 4]


# ---------- shanten / ukeire / yaku ----------

_SHANTEN = Shanten()
_CALC = HandCalculator()


def _shanten_with_melds(hand34: list[int], num_melds: int) -> int:
    # mahjong's calculate_shanten expects exactly 13 (or 14) tiles in the
    # 34-array. Open melds remove 3 tiles each from the concealed count.
    # We replicate the meld presence by padding with ghost honor triplets
    # so the algorithm sees a complete-shaped hand.
    #
    # Validity check is on `concealed + 3 * num_melds`, NOT raw concealed
    # count — with N open melds the concealed portion is only 13 - 3N
    # (or 14 - 3N right after a draw / call).
    total_with_melds = sum(hand34) + num_melds * 3
    if total_with_melds not in (13, 14):
        return 8  # malformed — return a deliberately bad score
    if num_melds == 0:
        return _SHANTEN.calculate_shanten(list(hand34))
    padded = list(hand34)
    ghost_slots = [27, 28, 29, 30, 31, 32, 33]
    placed = 0
    for h in ghost_slots:
        if placed >= num_melds:
            break
        if padded[h] == 0:
            padded[h] = 3
            placed += 1
    if placed < num_melds:
        return 8
    return _SHANTEN.calculate_shanten(padded)


def _ukeire(hand34: list[int], num_melds: int) -> list[int]:
    """Indices of tiles that strictly reduce shanten when added."""
    sh = _shanten_with_melds(hand34, num_melds)
    out = []
    for idx in range(34):
        if hand34[idx] >= 4:
            continue
        hand34[idx] += 1
        new_sh = _shanten_with_melds(hand34, num_melds)
        hand34[idx] -= 1
        if new_sh < sh:
            out.append(idx)
    return out


def _tenpai_waits(hand34: list[int], num_melds: int) -> list[int]:
    """If hand is tenpai (shanten == 0 with 13 tiles), return wait tiles."""
    sh = _shanten_with_melds(hand34, num_melds)
    if sh != 0:
        return []
    out = []
    for idx in range(34):
        if hand34[idx] >= 4:
            continue
        hand34[idx] += 1
        new_sh = _shanten_with_melds(hand34, num_melds)
        hand34[idx] -= 1
        if new_sh == -1:
            out.append(idx)
    return out


def _hand_value(state: State, hand34_full: list[int], aka_in_hand: dict[str, int],
                win_tile_idx: int, is_tsumo: bool):
    """Call mahjong's HandCalculator. hand34_full must include the
    winning tile and total to 14 tiles (concealed + meld). Returns the
    HandResponse — `.cost is not None` ⇔ valid (yaku present)."""
    tiles136 = _hand34_to_136(hand34_full, aka_in_hand)
    win_tile_136 = _tile136_for_index(win_tile_idx)
    melds = [_meld_to_mahjong(m) for m in state.melds if m["type"] != "ankan"]
    ankan_melds = [_meld_to_mahjong(m) for m in state.melds if m["type"] == "ankan"]
    # mahjong wants ankan in melds list with opened=False; pass them all
    melds.extend(ankan_melds)
    dora_136 = [_tile136_for_index(_to34(t)) for t in state.dora_indicators]
    config = HandConfig(
        is_tsumo=is_tsumo,
        is_riichi=state.reach_accepted and is_tsumo,
        player_wind=seat_wind_const(state.actor_id, state.oya),
        round_wind={"E": EAST, "S": SOUTH, "W": WEST}.get(state.bakaze, EAST),
    )
    try:
        return _CALC.estimate_hand_value(
            tiles=tiles136,
            win_tile=win_tile_136,
            melds=melds if melds else None,
            dora_indicators=dora_136 if dora_136 else None,
            config=config,
        )
    except Exception:
        return None


def _has_obvious_yaku_path(
    state: State,
    hand34: list[int],
    extra_melds: list[dict] | None = None,
) -> bool:
    """Cheap heuristic: hand has a likely yaku path. Mirrors the C++
    `has_obvious_yaku_path`. `extra_melds` are hypothetical melds that
    haven't been added to `state.melds` yet (e.g. the pon we're
    considering)."""
    extras = extra_melds or []
    all_melds = list(state.melds) + extras
    sw = seat_wind_const(state.actor_id, state.oya) - EAST + 27  # 27..30
    rw = _to34(state.bakaze)

    def is_yakuhai(idx: int) -> bool:
        return idx >= 31 or idx == sw or idx == rw

    # Open meld of yakuhai
    for m in all_melds:
        if m["type"] != "chi" and is_yakuhai(_to34(m["tiles"][0])):
            return True
    # Concealed triplet of yakuhai
    for idx in range(34):
        if hand34[idx] >= 3 and is_yakuhai(idx):
            return True
    # Tanyao path
    all_simple = True
    for idx in range(34):
        if hand34[idx] > 0 and _is_yaochuu(idx):
            all_simple = False
            break
    if all_simple:
        for m in all_melds:
            for t in m["tiles"]:
                if _is_yaochuu(_to34(t)):
                    all_simple = False
                    break
            if not all_simple:
                break
    if all_simple:
        return True
    # Single-suit progress (honitsu/chinitsu)
    has_suit = [False, False, False]
    for idx in range(27):
        if hand34[idx] > 0:
            has_suit[idx // 9] = True
    for m in all_melds:
        for t in m["tiles"]:
            i = _to34(t)
            if i < 27:
                has_suit[i // 9] = True
    if sum(has_suit) <= 1:
        return True
    return False


def _has_viable_yaku(
    state: State,
    hand13_34: list[int],
    aka: dict[str, int],
    extra_melds: list[dict] | None = None,
) -> bool:
    """At tenpai, try each wait via HandCalculator; otherwise heuristic.
    `extra_melds` lets callers evaluate a hypothetical post-call hand
    without mutating `state.melds`."""
    extras = extra_melds or []
    num_melds = len(state.melds) + len(extras)
    sh = _shanten_with_melds(hand13_34, num_melds)
    if sh == 0:
        # Tenpai: try each wait. We don't pass extras to HandCalculator
        # because constructing the right 136-array with hypothetical
        # melds is fiddly; the heuristic path below is good enough for
        # the rare "tenpai with a hypothetical call" case (it would
        # only matter if you could call into agari, which mjai handles
        # via daiminkan→rinshan separately).
        for w in _tenpai_waits(hand13_34, num_melds):
            if extras:
                if _has_obvious_yaku_path(state, hand13_34, extras):
                    return True
                continue
            full = list(hand13_34)
            full[w] += 1
            res = _hand_value(state, full, aka, w, is_tsumo=False)
            if res is not None and res.cost is not None:
                return True
        return False
    return _has_obvious_yaku_path(state, hand13_34, extras)


def _effective_ukeire(state: State, hand13_34: list[int], num_melds: int) -> int:
    total = 0
    for idx in _ukeire(hand13_34, num_melds):
        rem = 4 - state.visible_count(idx)
        if rem > 0:
            total += rem
    return total


def _wait_remaining(state: State, waits: list[int], hand13_34: list[int]) -> int:
    total = 0
    for idx in waits:
        rem = 4 - state.visible_count(idx)
        extra = state.hand34[idx] - hand13_34[idx]
        if rem + extra > 0:
            total += rem + max(0, extra)
    return total


def _hand_dora_count(state: State, hand34: list[int]) -> int:
    doras = _dora_indices(state)
    total = sum(hand34[idx] for idx in doras)
    for suit, idx in (("m", 4), ("p", 13), ("s", 22)):
        total += min(state.aka_in_hand.get(suit, 0), hand34[idx])
    return total


def _riichi_value_score(state: State, hand13_34: list[int], waits: list[int]) -> int:
    dora = _hand_dora_count(state, hand13_34)
    score = 12 + dora * 14
    if _seat_is_oya(state.actor_id, state):
        score += 8
    if len(waits) >= 3:
        score += 12
    score += min(18, _wait_remaining(state, waits, hand13_34) * 3)
    return score


def _should_damaten(
    state: State,
    value_score: int,
    waits: int,
    live_waits: int,
    declaration_danger: int,
) -> bool:
    """Prefer closed tenpai without riichi when the stick/risk is not worth it."""
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return False
    if style == _STYLE_TANYAO_FAST:
        return False
    if style == _STYLE_BALANCED:
        if state.rivals_riichi and declaration_danger >= max(12, _fold_trigger_danger(state) // 2):
            return True
        if value_score >= _damaten_value_score(state):
            return True
        if state.left_tiles < _min_left_for_riichi(state) + 8:
            return True
    if style == _STYLE_CONSERVATIVE:
        if state.rivals_riichi or declaration_danger >= _fold_trigger_danger(state):
            return True
        if state.left_tiles < _min_left_for_riichi(state) + 8:
            return True
    if state.rivals_riichi and declaration_danger >= _fold_trigger_danger(state):
        return True
    thin_wall = state.left_tiles < _min_left_for_riichi(state) + 4
    if thin_wall and value_score >= _riichi_min_score(state):
        return True
    if _score_table_pressure(state) and value_score >= _damaten_value_score(state):
        return True
    if (
        value_score >= _damaten_value_score(state) + 8
        and waits >= 2
        and live_waits >= 3
        and (state.rivals_riichi or _score_table_pressure(state))
    ):
        return True
    return False


def _discard_value_penalty(state: State, tile_idx: int, hand_before_discard: list[int]) -> int:
    penalty = 0
    if tile_idx in _dora_indices(state):
        penalty += 18
    if tile_idx in (4, 13, 22):
        suit = "mps"[tile_idx // 9]
        if state.aka_in_hand.get(suit, 0) > 0:
            penalty += 16
    if _is_yakuhai_idx(state, tile_idx) and hand_before_discard[tile_idx] >= 2:
        penalty += 8
    if tile_idx < 27 and 1 <= tile_idx % 9 <= 7:
        left = hand_before_discard[tile_idx - 1] if tile_idx % 9 > 0 else 0
        right = hand_before_discard[tile_idx + 1] if tile_idx % 9 < 8 else 0
        if left and right:
            penalty += 7
    return penalty


def _danger_tier(state: State, danger: int) -> int:
    """Coarse safety bucket; value sorting only wins inside the same bucket."""
    style = _play_style(state)
    if style == _STYLE_AGGRESSIVE:
        return 0
    if danger <= 0:
        return 0
    if style == _STYLE_CONSERVATIVE:
        if danger < 8:
            return 1
        if danger < _fold_trigger_danger(state):
            return 2
        return 3
    if danger < 20:
        return 1
    if danger < _fold_trigger_danger(state):
        return 2
    return 3


def _shape_score(hand34: list[int]) -> int:
    """Rough shape quality: pairs, ryanmen, and useful compound shapes."""
    score = 0
    for idx in range(34):
        if hand34[idx] >= 2:
            score += 3
        if hand34[idx] >= 3:
            score += 2
    for suit in range(3):
        base = suit * 9
        for r in range(8):
            a = hand34[base + r]
            b = hand34[base + r + 1]
            if a and b:
                score += 6 if 1 <= r <= 5 else 3
        for r in range(7):
            if hand34[base + r] and hand34[base + r + 2]:
                score += 2
        for r in range(6):
            if hand34[base + r] and hand34[base + r + 1] and hand34[base + r + 2]:
                score += 5
        for r in range(5):
            if (
                hand34[base + r]
                and hand34[base + r + 1]
                and hand34[base + r + 2]
                and hand34[base + r + 3]
            ):
                score += 4
    return score


def _target_yaku(state: State, hand34: list[int], extra_melds: list[dict] | None = None) -> str:
    if _play_style(state) == _STYLE_TANYAO_FAST:
        return "tanyao"
    melds = list(state.melds) + (extra_melds or [])
    for m in melds:
        if m["type"] != "chi" and _is_yakuhai_idx(state, _to34(m["tiles"][0])):
            return "yakuhai"
    for idx in range(34):
        if hand34[idx] >= 2 and _is_yakuhai_idx(state, idx):
            return "yakuhai"
    simple_tiles = sum(hand34[:27])
    yaochuu_tiles = sum(hand34[i] for i in range(34) if _is_yaochuu(i))
    if simple_tiles and yaochuu_tiles <= 1:
        return "tanyao"
    suit_counts = [sum(hand34[s * 9:s * 9 + 9]) for s in range(3)]
    for m in melds:
        for t in m["tiles"]:
            i = _to34(t)
            if i < 27:
                suit_counts[i // 9] += 1
    if max(suit_counts) >= max(7, sum(suit_counts) - 2):
        return "honitsu"
    tripletish = sum(1 for n in hand34 if n >= 2)
    if tripletish >= 5:
        return "toitoi"
    return "form"


def _hand_value_potential(
    state: State,
    hand34: list[int],
    extra_melds: list[dict] | None = None,
) -> int:
    """Rough value path score used after safety/shanten are already gated."""
    target = _target_yaku(state, hand34, extra_melds)
    style = _play_style(state)
    if style == _STYLE_TANYAO_FAST:
        yaochuu = sum(hand34[i] for i in range(34) if _is_yaochuu(i))
        simple = sum(hand34[:27]) - sum(hand34[i] for i in range(27) if _is_yaochuu(i))
        return simple * 5 - yaochuu * 28 + _effective_ukeire(state, hand34, len(state.melds) + len(extra_melds or []))
    score = _hand_dora_count(state, hand34) * 18
    if _seat_is_oya(state.actor_id, state):
        score += 5
    if target == "yakuhai":
        score += 18
    elif target == "honitsu":
        score += 22
    elif target == "toitoi":
        score += 16
    elif target == "tanyao":
        score += 10
    for idx in range(34):
        if _is_yakuhai_idx(state, idx):
            if hand34[idx] >= 3:
                score += 12
            elif hand34[idx] >= 2:
                score += 8
        elif hand34[idx] >= 3:
            score += 4
    score += min(18, _shape_score(hand34) // 3)
    if style == _STYLE_AGGRESSIVE:
        return score * 2
    if style == _STYLE_CONSERVATIVE:
        return score // 2
    return score


def _call_plan_penalty(state: State, meld_type: str, called_idx: int, post: list[int], hyp: list[dict]) -> int:
    target = _target_yaku(state, post, hyp)
    if target == "yakuhai":
        return 0 if meld_type == "pon" and _is_yakuhai_idx(state, called_idx) else 8
    if target == "tanyao":
        return 0 if called_idx < 27 and not _is_yaochuu(called_idx) else 12
    if target == "honitsu":
        return 0 if called_idx >= 27 or sum(1 for x in post[:27] if x > 0) <= 9 else 6
    if target == "toitoi":
        return 0 if meld_type == "pon" else 14
    return 4 if meld_type == "chi" else 2


# ---------- mjai actions ----------

def _act_none(actor: int) -> dict:
    return {"type": "none"}


def _pon_consumed_options(state: State, called_idx: int) -> list[list[str]]:
    """Legal [tile, tile] pairs from hand to pon `called_idx` (mjai strings)."""
    if state.hand34[called_idx] < 2:
        return []
    base = _from34(called_idx)
    if not (base[0] == "5" and base[1] in "mps"):
        return [[base, base]]
    suit = base[1]
    total = state.hand34[called_idx]
    red = min(state.aka_in_hand.get(suit, 0), total)
    plain = total - red
    pr = f"5{suit}r"
    opts: list[list[str]] = []
    seen: set[tuple[str, str]] = set()

    def add(pair: list[str]) -> None:
        k = (pair[0], pair[1])
        if k not in seen:
            seen.add(k)
            opts.append(pair)

    if plain >= 2:
        add([base, base])
    if red >= 1 and plain >= 1:
        add([pr, base])
    if red >= 2:
        add([pr, pr])
    if not opts and total >= 2:
        if red >= 1:
            add([pr, base])
        elif plain >= 2:
            add([base, base])
    return opts if opts else [[base, base]]


def _tile_name_options_for_chi(state: State, idx: int) -> list[str]:
    base = _from34(idx)
    if not (idx in (4, 13, 22) and state.aka_in_hand.get(base[1], 0) > 0):
        return [base]
    plain = state.hand34[idx] - state.aka_in_hand.get(base[1], 0)
    if plain > 0:
        return [base, f"5{base[1]}r"]
    return [f"5{base[1]}r"]


def _pick_consumed_for_daiminkan(state: State, called_idx: int) -> list[str] | None:
    if state.hand34[called_idx] < 3:
        return None
    base = _from34(called_idx)
    if base[0] == "5" and base[1] in "mps":
        suit = base[1]
        if state.aka_in_hand.get(suit, 0) >= 1:
            return [f"5{suit}r", base, base]
    return [base, base, base]


def _try_kita_after_tsumo(state: State, hand14: list[int], actor: int) -> dict | None:
    """Sanma: declare kita if value gain is worth the shape cost."""
    if state.num_players != 3 or state.reach_accepted or hand14[30] < 1:
        return None
    if state.rivals_riichi or _score_table_pressure(state):
        return None
    h = list(hand14)
    h[30] -= 1
    nm = len(state.melds)
    tot = sum(h) + nm * 3
    if tot not in (13, 14):
        return None
    sh_before = _shanten_with_melds(hand14, nm)
    sh_after = _shanten_with_melds(h, nm)
    if sh_after <= sh_before:
        return {"type": "kita", "actor": actor, "pai": "N"}
    if sh_before <= 1 and sh_after == sh_before + 1:
        return {"type": "kita", "actor": actor, "pai": "N"}
    return None


def _chi_options(state: State, called_idx: int) -> list[list[str]]:
    """Possible chi consumed-pair lists for called_idx (mjai strings)."""
    if called_idx >= 27:
        return []
    n = called_idx % 9 + 1
    pairs = []
    for a, b in [(n - 2, n - 1), (n - 1, n + 1), (n + 1, n + 2)]:
        if a < 1 or b > 9 or a > 9 or b < 1:
            continue
        idx_a = (called_idx // 9) * 9 + (a - 1)
        idx_b = (called_idx // 9) * 9 + (b - 1)
        if state.hand34[idx_a] < 1 or state.hand34[idx_b] < 1:
            continue
        for name_a in _tile_name_options_for_chi(state, idx_a):
            for name_b in _tile_name_options_for_chi(state, idx_b):
                pairs.append([name_a, name_b])
    return pairs


# ---------- decision: own tsumo ----------

def decide_after_own_tsumo(state: State, last_event: dict) -> dict:
    actor = state.actor_id
    drawn = last_event["pai"]
    drawn_idx = _to34(drawn)

    # Rule 0: Tsumo agari
    hand14 = list(state.hand34)
    safety = _safety_inventory(state, hand14)
    low_safety = safety["usable"] <= _min_safe_tiles_to_fold(state)
    if _shanten_with_melds(hand14, len(state.melds)) == -1:
        res = _hand_value(state, hand14, state.aka_in_hand, drawn_idx, is_tsumo=True)
        if res is not None and res.cost is not None:
            return _with_meta(
                {"type": "hora", "actor": actor, "target": actor, "pai": drawn},
                state,
                "tsumo",
                safety=safety,
            )

    kita_act = _try_kita_after_tsumo(state, hand14, actor)
    if kita_act is not None:
        return _with_meta(kita_act, state, "kita", safety=safety)

    # Rule: Ankan / Kakan if shanten doesn't worsen — skip under defence (new dora risk).
    if not state.reach_accepted and not _avoid_kan_for_defense(state):
        sh14 = _shanten_with_melds(hand14, len(state.melds))
        for idx in range(34):
            if hand14[idx] >= 4:
                # Build the post-ankan concealed hand and re-check shanten
                test = list(hand14)
                test[idx] = 0
                aka_keep = dict(state.aka_in_hand)
                if idx in (4, 13, 22):
                    aka_keep["mps"[idx // 9]] = 0
                # Treat ankan as one more meld for shanten purposes
                if _shanten_with_melds(test, len(state.melds) + 1) <= sh14:
                    base = _from34(idx)
                    consumed = [base, base, base, base]
                    if base[0] == "5" and base[1] in "mps":
                        suit = base[1]
                        if state.aka_in_hand.get(suit, 0) >= 1:
                            consumed = [f"5{suit}r", base, base, base]
                    return _with_meta(
                        {"type": "ankan", "actor": actor, "consumed": consumed},
                        state,
                        "ankan",
                        shanten=sh14,
                        safety=safety,
                    )

        # Rule: Kakan if shanten doesn't worsen
        for m in state.melds:
            if m["type"] != "pon":
                continue
            tile = m["tiles"][0]
            tidx = _to34(tile)
            if hand14[tidx] >= 1:
                test = list(hand14)
                test[tidx] -= 1
                if _shanten_with_melds(test, len(state.melds)) <= sh14:
                    pai = _from34(tidx)
                    if pai[0] == "5" and pai[1] in "mps" and state.aka_in_hand.get(pai[1], 0) > 0:
                        pai = f"5{pai[1]}r"
                    return _with_meta({
                        "type": "kakan",
                        "actor": actor,
                        "pai": pai,
                        "consumed": list(m["tiles"]),
                    }, state, "kakan", shanten=sh14, safety=safety)

    # Already in riichi → tsumogiri
    if state.reach_accepted:
        return _with_meta({
            "type": "dahai",
            "actor": actor,
            "pai": drawn,
            "tsumogiri": True,
        }, state, "riichi_tsumogiri", safety=safety)

    # Rule 3: Riichi by waits, rough value, and declaration safety.
    style = _play_style(state)
    best_riichi: tuple[tuple[int, int, int, int], str, int, int] | None = None
    if state.is_closed() and not state.reach_declared and state.left_tiles >= _min_left_for_riichi(state):
        for idx in range(34):
            if hand14[idx] == 0:
                continue
            test = list(hand14)
            test[idx] -= 1
            if _shanten_with_melds(test, len(state.melds)) != 0:
                continue
            waits = _tenpai_waits(test, len(state.melds))
            nw = len(waits)
            if nw == 0:
                continue
            value_score = _riichi_value_score(state, test, waits)
            decl_danger = _discard_danger(idx, state) if state.rivals_riichi else 0
            remain = _wait_remaining(state, waits, test)
            pai = _from34(idx)
            if idx in (4, 13, 22):
                suit = "mps"[idx // 9]
                # If we ONLY have aka, discard aka
                if state.aka_in_hand.get(suit, 0) > 0 and hand14[idx] - state.aka_in_hand.get(suit, 0) == 0:
                    pai = f"5{suit}r"
            rank = (value_score, nw, remain, -decl_danger)
            if best_riichi is None or rank > best_riichi[0]:
                best_riichi = (rank, pai, nw, decl_danger)
        if best_riichi and best_riichi[2] >= 3:
            value_score, _nw_score, live_waits, _neg_danger = best_riichi[0]
            nw = best_riichi[2]
            decl_danger = best_riichi[3]
            # Stricter when someone else is already riichi or the wall is thin.
            allow_riichi = value_score >= _riichi_min_score(state)
            if state.rivals_riichi:
                can_push_decl = low_safety and value_score >= _damaten_value_score(state) + 8
                allow_riichi = allow_riichi and (
                    decl_danger < _fold_trigger_danger(state) or can_push_decl
                )
                allow_riichi = allow_riichi and nw >= 3
            elif _score_table_pressure(state):
                allow_riichi = allow_riichi and nw >= 3
            if allow_riichi and _should_damaten(state, value_score, nw, live_waits, decl_danger):
                allow_riichi = False
            if allow_riichi:
                return _with_meta(
                    {"type": "reach", "actor": actor},
                    state,
                    "riichi_push_low_safety" if low_safety and state.rivals_riichi else "riichi_value",
                    value_score=value_score,
                    waits=nw,
                    live_waits=live_waits,
                    declaration_danger=decl_danger,
                    safety=safety,
                )

    # Rule 1: Discard — minimise shanten; under defence prefer safer tile, then ukeire.
    # Optional fold: allow +1 shanten if it drops danger enough (兜).
    use_defense = (
        bool(state.rivals_riichi) or _score_table_pressure(state) or style == _STYLE_CONSERVATIVE
    ) and not state.reach_accepted and style not in {_STYLE_AGGRESSIVE, _STYLE_TANYAO_FAST}
    cands: list[tuple[int, int, int, int, int, int, str, int]] = []
    for idx in range(34):
        if hand14[idx] == 0:
            continue
        test = list(hand14)
        test[idx] -= 1
        sh = _shanten_with_melds(test, len(state.melds))
        uke = _effective_ukeire(state, test, len(state.melds))
        shape = _shape_score(test)
        value = _hand_value_potential(state, test) - min(
            _call_value_penalty_max(state),
            _discard_value_penalty(state, idx, hand14),
        )
        pai = _from34(idx)
        if idx in (4, 13, 22):
            suit = "mps"[idx // 9]
            if state.aka_in_hand.get(suit, 0) > 0 and hand14[idx] - state.aka_in_hand.get(suit, 0) == 0:
                pai = f"5{suit}r"
        dan = _discard_danger(idx, state) if use_defense else 0
        cands.append((sh, _danger_tier(state, dan), -value, dan, -uke, -shape, pai, idx))
    if not cands:
        return _act_none(actor)
    min_sh = min(c[0] for c in cands)
    if style == _STYLE_TANYAO_FAST:
        best = min(cands, key=lambda c: (c[0], c[2], c[4], c[5], c[7]))
    elif style == _STYLE_CONSERVATIVE and use_defense:
        best = min(cands, key=lambda c: (c[1], c[3], c[0], c[2], c[4], c[5], c[7]))
    else:
        same_layer = [c for c in cands if c[0] == min_sh]
        best = min(same_layer)
    if use_defense and style != _STYLE_CONSERVATIVE:
        fold_layer = [c for c in cands if c[0] == min_sh + 1]
        if fold_layer:
            best_fold = min(fold_layer)
            fold_trigger = _fold_trigger_danger(state) + (12 if low_safety and min_sh <= 1 else 0)
            if best[3] >= fold_trigger and (best[3] - best_fold[3]) >= _fold_min_danger_drop(state):
                best = best_fold
    sh, tier, neg_value, dan, neg_u, neg_shape, pai, idx = best
    # Detect tsumogiri: discard equals drawn tile
    tsumogiri = pai == drawn
    if min_sh == 0 and best_riichi is not None:
        reason = "damaten"
    elif use_defense and low_safety and sh == min_sh and style != _STYLE_CONSERVATIVE:
        reason = "push_low_safety"
    elif use_defense and sh > min_sh:
        reason = "fold"
    elif use_defense:
        reason = "defensive_discard"
    elif style == _STYLE_TANYAO_FAST:
        reason = "tanyao_fast"
    else:
        reason = "shape_discard"
    return _with_meta({
        "type": "dahai",
        "actor": actor,
        "pai": pai,
        "tsumogiri": tsumogiri,
    }, state, reason, shanten=sh, safety_tier=tier, danger=dan, value_score=-neg_value, ukeire=-neg_u, shape=-neg_shape, safety=safety)


# ---------- decision: response to others ----------

def decide_after_others_dahai(state: State, last_event: dict) -> dict:
    actor = state.actor_id
    discarder = last_event["actor"]
    discarded = last_event["pai"]
    didx = _to34(discarded)
    safety = _safety_inventory(state)

    # Furiten skip: if we already discarded the wait tile.
    own_river_idxs = {_to34(t) for t in state.rivers[actor]}

    # Rule 0: Ron
    hand14 = list(state.hand34)
    hand14[didx] += 1
    if _shanten_with_melds(hand14, len(state.melds)) == -1 and didx not in own_river_idxs:
        # Build a temporary aka view for the call (no aka added by someone else's discard,
        # unless the discarded tile itself is an aka — track via mjai tile string)
        aka_in = dict(state.aka_in_hand)
        if discarded.endswith("r"):
            aka_in[_normalize(discarded)[1]] = aka_in.get(_normalize(discarded)[1], 0) + 1
        res = _hand_value(state, hand14, aka_in, didx, is_tsumo=False)
        if res is not None and res.cost is not None:
            return _with_meta({
                "type": "hora",
                "actor": actor,
                "target": discarder,
                "pai": discarded,
            }, state, "ron", safety=safety)

    if state.reach_accepted:
        # In riichi we cannot call. (Ankan would be allowed but we
        # already reach-accepted → no upgrade here.)
        return _act_none(actor)

    current_sh = _shanten_with_melds(state.hand34, len(state.melds))

    # ----- Daiminkan -----
    consumed = _pick_consumed_for_daiminkan(state, didx)
    if consumed is not None:
        post = list(state.hand34)
        post[didx] -= 3
        new_sh = _shanten_with_melds(post, len(state.melds) + 1)
        hypothetical = [{
            "type": "daiminkan",
            "tiles": [discarded] + consumed,
            "from_seat": discarder,
            "called_tile": discarded,
        }]
        if new_sh < current_sh and _has_viable_yaku(
            state, post, state.aka_in_hand, extra_melds=hypothetical
        ) and _allow_open_call(state, "daiminkan", didx, current_sh, new_sh):
            return _with_meta({
                "type": "daiminkan",
                "actor": actor,
                "target": discarder,
                "pai": discarded,
                "consumed": consumed,
            }, state, "daiminkan", shanten=new_sh, target_yaku=_target_yaku(state, post, hypothetical), safety=safety)

    # ----- Pon — pick best consumption when red-5 variants exist -----
    use_def_pon = bool(state.rivals_riichi) or _score_table_pressure(state)
    best_pon: tuple | None = None
    for pon_consumed in _pon_consumed_options(state, didx):
        post = list(state.hand34)
        for c in pon_consumed:
            post[_to34(c)] -= 1
        best_inner: tuple | None = None
        best_sh = 99
        best_post: list[int] | None = None
        for j in range(34):
            if post[j] == 0:
                continue
            t = list(post)
            t[j] -= 1
            sh = _shanten_with_melds(t, len(state.melds) + 1)
            dan = _discard_danger(j, state) if use_def_pon else 0
            val = min(_call_value_penalty_max(state), _discard_value_penalty(state, j, post))
            uke = _effective_ukeire(state, t, len(state.melds) + 1)
            shape = _shape_score(t)
            value = _hand_value_potential(state, t) - val
            key = (sh, _danger_tier(state, dan), -value, dan, val, -uke, -shape, j)
            if best_inner is None or key < best_inner:
                best_inner = key
                best_sh = sh
                best_post = t
        hypothetical = [{
            "type": "pon",
            "tiles": list(pon_consumed) + [discarded],
            "from_seat": discarder,
            "called_tile": discarded,
        }]
        if best_inner is None or best_post is None or best_sh >= current_sh:
            continue
        if not _has_viable_yaku(
            state, best_post, state.aka_in_hand, extra_melds=hypothetical
        ) or not _allow_open_call(state, "pon", didx, current_sh, best_sh, best_inner[3]):
            continue
        plan0 = _call_plan_penalty(state, "pon", didx, best_post, hypothetical)
        sh0, tier0, neg_value0, dan0, val0, neg_u0, neg_shape0, _j0 = best_inner
        rank = (sh0, tier0, neg_value0, dan0, val0 + plan0, neg_u0, neg_shape0, str(pon_consumed))
        if best_pon is None or rank < best_pon[0]:
            best_pon = (rank, pon_consumed, hypothetical)
    if best_pon is not None:
        r, pon_consumed, hyp = best_pon
        return _with_meta({
            "type": "pon",
            "actor": actor,
            "target": discarder,
            "pai": discarded,
            "consumed": pon_consumed,
        }, state, "target_yaku_call", shanten=r[0], safety_tier=r[1], value_score=-r[2], danger=r[3], value_penalty=r[4], target_yaku=_target_yaku(state, state.hand34, hyp), safety=safety)

    # ----- Chi (only from kamicha) — pick best chi shape by shanten, danger, ukeire -----
    if discarder == state.kamicha():
        use_def = bool(state.rivals_riichi) or _score_table_pressure(state)
        best_chi: tuple | None = None  # (sh, dan, -uke, chi_consumed list, hyp dict)
        for chi_consumed in _chi_options(state, didx):
            post = list(state.hand34)
            for c in chi_consumed:
                post[_to34(c)] -= 1
            best_inner: tuple | None = None
            best_sh = 99
            best_post: list[int] | None = None
            for j in range(34):
                if post[j] == 0:
                    continue
                t = list(post)
                t[j] -= 1
                sh = _shanten_with_melds(t, len(state.melds) + 1)
                dan = _discard_danger(j, state) if use_def else 0
                val = min(_call_value_penalty_max(state), _discard_value_penalty(state, j, post))
                uke = _effective_ukeire(state, t, len(state.melds) + 1)
                shape = _shape_score(t)
                value = _hand_value_potential(state, t) - val
                key = (sh, _danger_tier(state, dan), -value, dan, val, -uke, -shape, j)
                if best_inner is None or key < best_inner:
                    best_inner = key
                    best_sh = sh
                    best_post = t
            hypothetical = [{
                "type": "chi",
                "tiles": list(chi_consumed) + [discarded],
                "from_seat": discarder,
                "called_tile": discarded,
            }]
            if best_inner is None or best_post is None or best_sh >= current_sh:
                continue
            if not _has_viable_yaku(
                state, best_post, state.aka_in_hand, extra_melds=hypothetical
            ) or not _allow_open_call(state, "chi", didx, current_sh, best_sh, best_inner[3]):
                continue
            plan0 = _call_plan_penalty(state, "chi", didx, best_post, hypothetical)
            sh0, tier0, neg_value0, dan0, val0, neg_u0, neg_shape0, _j0 = best_inner
            rank = (sh0, tier0, neg_value0, dan0, val0 + plan0, neg_u0, neg_shape0, str(chi_consumed))
            if best_chi is None or rank < best_chi[0]:
                best_chi = (rank, chi_consumed, hypothetical)
        if best_chi is not None:
            r, chi_consumed, hyp = best_chi
            return _with_meta({
                "type": "chi",
                "actor": actor,
                "target": discarder,
                "pai": discarded,
                "consumed": chi_consumed,
            }, state, "target_yaku_call", shanten=r[0], safety_tier=r[1], value_score=-r[2], danger=r[3], value_penalty=r[4], target_yaku=_target_yaku(state, state.hand34, hyp), safety=safety)

    return _act_none(actor)


def decide_after_others_kakan(state: State, last_event: dict) -> dict:
    """Chankan window. Same as ron but on kakan tile."""
    actor = state.actor_id
    declarer = last_event["actor"]
    tile = last_event["pai"]
    tidx = _to34(tile)
    own_river_idxs = {_to34(t) for t in state.rivers[actor]}
    hand14 = list(state.hand34)
    hand14[tidx] += 1
    if _shanten_with_melds(hand14, len(state.melds)) == -1 and tidx not in own_river_idxs:
        aka_in = dict(state.aka_in_hand)
        if tile.endswith("r"):
            aka_in[_normalize(tile)[1]] = aka_in.get(_normalize(tile)[1], 0) + 1
        res = _hand_value(state, hand14, aka_in, tidx, is_tsumo=False)
        if res is not None and res.cost is not None:
            return _with_meta({
                "type": "hora",
                "actor": actor,
                "target": declarer,
                "pai": tile,
            }, state, "chankan")
    return _act_none(actor)


# ---------- top-level ----------

def react(state: State, events_json: str) -> str:
    events = json.loads(events_json)
    if not events:
        return json.dumps({"type": "none"}, separators=(",", ":"))
    for e in events:
        state.consume(e)
    last = events[-1]
    t = last.get("type")
    actor = state.actor_id
    if t == "tsumo" and last.get("actor") == actor:
        action = decide_after_own_tsumo(state, last)
    elif t == "dahai" and last.get("actor") != actor:
        action = decide_after_others_dahai(state, last)
    elif t == "kakan" and last.get("actor") != actor:
        action = decide_after_others_kakan(state, last)
    else:
        action = {"type": "none"}
    _record_experience_action(state, action)
    _record_experience_events(state, events)
    if any(e.get("type") == "end_game" for e in events):
        _finalize_experience(state)
    return json.dumps(action, separators=(",", ":"))


def _initial_actor_id() -> int:
    if len(sys.argv) > 1:
        try:
            return int(sys.argv[1])
        except ValueError:
            pass
    env = os.environ.get("AKAGI_PLAYER_ID")
    if env is not None:
        try:
            return int(env)
        except ValueError:
            pass
    return 0


def main() -> None:
    state = State(actor_id=_initial_actor_id())
    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            resp = react(state, line)
        except Exception as e:  # never crash the loop
            sys.stderr.write(f"bot error: {e}\n")
            sys.stderr.flush()
            resp = json.dumps({"type": "none"}, separators=(",", ":"))
        sys.stdout.write(resp + "\n")
        sys.stdout.flush()
        try:
            evs = json.loads(line)
        except Exception:
            continue
        if any(ev.get("type") == "end_game" for ev in evs):
            break


if __name__ == "__main__":
    main()
