import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from policy_guard import ConservativePolicyGuard, _to34


class FakePlayerState:
    wait_idx = 0

    def __init__(self, player_id: int) -> None:
        self.player_id = player_id
        self.waits = [False] * 34

    def update(self, raw: str) -> None:
        event = json.loads(raw)
        if event.get("type") == "dahai" and event.get("actor") == self.player_id:
            self.waits = [False] * 34
            self.waits[self.wait_idx] = True


def make_guard(tehais: list[str]) -> ConservativePolicyGuard:
    guard = ConservativePolicyGuard(
        FakePlayerState,
        default_num_players=4,
        initial_left_tiles=70,
        fast_riichi_left_tiles=50,
        very_fast_riichi_left_tiles=60,
    )
    for event in (
        {"type": "start_game", "id": 0},
        {
            "type": "start_kyoku",
            "num_players": 4,
            "scores": [25000, 25000, 25000, 25000],
            "tehais": [tehais, ["?"] * 13, ["?"] * 13, ["?"] * 13],
        },
        {"type": "tsumo", "actor": 0, "pai": "2m"},
    ):
        guard.consume(event)
    return guard


def meta_for_allowed(*pairs: tuple[int, float]) -> dict:
    mask_bits = 0
    q_values = []
    for idx, score in sorted(pairs):
        mask_bits |= 1 << idx
        q_values.append(score)
    return {"mask_bits": mask_bits, "q_values": q_values}


class PolicyGuardTest(unittest.TestCase):
    def test_balanced_reach_suppressed_when_live_waits_below_three(self) -> None:
        guard = make_guard(["1m", "1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p"])

        action = guard.suppress_reach_if_needed(
            {"type": "reach", "actor": 0, "pai": "2m"},
            [],
            "balanced",
        )

        self.assertEqual(action["type"], "dahai")
        self.assertEqual(action["meta"]["policy_guard"]["reason"], "low_live_waits")
        self.assertEqual(action["meta"]["policy_guard"]["live_waits"], 2)

    def test_balanced_reach_at_three_live_waits_discards_when_auto_riichi_disabled(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])

        action = guard.suppress_reach_if_needed(
            {"type": "reach", "actor": 0, "pai": "2m"},
            [],
            "balanced",
        )

        self.assertEqual(action["type"], "dahai")
        self.assertEqual(action["pai"], "2m")
        self.assertEqual(action["meta"]["policy_guard"]["reason"], "auto_riichi_disabled")

    def test_reach_without_prefilled_discard_uses_best_meta_discard(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])

        action = guard.suppress_reach_if_needed(
            {
                "type": "reach",
                "actor": 0,
                "meta": meta_for_allowed((1, 0.2), (2, 0.7), (37, 0.9)),
            },
            [],
            "balanced",
        )

        self.assertEqual(action["type"], "dahai")
        self.assertEqual(action["pai"], "3m")
        self.assertEqual(action["meta"]["policy_guard"]["reason"], "missing_reach_discard")
        self.assertEqual(action["meta"]["policy_guard"]["fallback_dahai"], "3m")

    def test_reach_without_meta_discard_tsumogiri_fallback(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])

        action = guard.suppress_reach_if_needed(
            {"type": "reach", "actor": 0},
            [],
            "balanced",
        )

        self.assertEqual(action["type"], "dahai")
        self.assertEqual(action["pai"], "2m")
        self.assertTrue(action["tsumogiri"])
        self.assertEqual(
            action["meta"]["policy_guard"]["reason"],
            "missing_reach_discard_tsumogiri_fallback",
        )

    def test_conservative_late_game_prefers_genbutsu(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "9m"})
        guard.consume({"type": "dahai", "actor": 2, "pai": "9m"})
        guard.consume({"type": "dahai", "actor": 3, "pai": "9m"})
        guard.left_tiles = 20

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((8, 0.1), (13, 0.9)),
            },
            [],
            "conservative",
        )

        self.assertEqual(action["type"], "dahai")
        self.assertEqual(action["pai"], "9m")
        self.assertEqual(action["meta"]["policy_guard"]["reason"], "late_game_conservative")

    def test_late_game_without_riichi_keeps_model_honor_discard(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "3m"})
        guard.consume({"type": "dahai", "actor": 2, "pai": "3m"})
        guard.left_tiles = 9

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "N",
                "meta": meta_for_allowed((2, 0.1), (30, 0.9)),
            },
            [],
            "conservative",
        )

        self.assertEqual(action["pai"], "N")
        self.assertNotIn("policy_guard", action.get("meta", {}))

    def test_north_maps_to_honor_index(self) -> None:
        self.assertEqual(_to34("N"), 30)

    def test_single_visible_north_is_not_hard_safe_against_tenpai(self) -> None:
        guard = make_guard(["N", "N", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p"])
        guard.consume({"type": "dahai", "actor": 2, "pai": "N"})
        guard.consume({"type": "reach", "actor": 1})

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((13, 0.1), (30, 0.9)),
            },
            [],
            "conservative",
        )

        self.assertEqual(action["pai"], "5p")
        self.assertNotIn("policy_guard", action.get("meta", {}))

    def test_late_game_does_not_treat_one_discarded_honor_as_safe(self) -> None:
        guard = make_guard(["N", "N", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "N"})
        guard.left_tiles = 10

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((13, 0.1), (30, 0.9)),
            },
            [],
            "conservative",
        )

        self.assertEqual(action["pai"], "5p")
        self.assertNotIn("policy_guard", action.get("meta", {}))

    def test_conservative_before_twenty_tiles_keeps_model_discard(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "9m"})
        guard.left_tiles = 21

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((8, 0.1), (13, 0.9)),
            },
            [],
            "conservative",
        )

        self.assertEqual(action["pai"], "5p")
        self.assertNotIn("policy_guard", action.get("meta", {}))

    def test_balanced_early_game_keeps_model_discard(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "9m"})

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((8, 0.1), (13, 0.9)),
            },
            [],
            "balanced",
        )

        self.assertEqual(action["pai"], "5p")
        self.assertNotIn("policy_guard", action.get("meta", {}))

    def test_opponent_riichi_triggers_genbutsu_defense(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "9m"})
        guard.consume({"type": "reach", "actor": 1})

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((8, 0.1), (13, 0.9)),
            },
            [],
            "balanced",
        )

        self.assertEqual(action["pai"], "9m")
        self.assertEqual(action["meta"]["policy_guard"]["reason"], "riichi_defense")

    def test_hot_opponent_does_not_force_opening_betaori(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "9m"})
        guard.hot_opponent_seat = 1
        guard.left_tiles = 62

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5p",
                "meta": meta_for_allowed((8, 0.1), (13, 0.9)),
            },
            [],
            "conservative",
        )

        self.assertEqual(action["pai"], "5p")
        self.assertNotIn("policy_guard", action.get("meta", {}))

    def test_defensive_tie_prefers_normal_five_over_red_five(self) -> None:
        guard = make_guard(["1m", "3m", "4m", "5m", "6m", "7m", "8m", "9m", "1p", "2p", "3p", "4p", "5p"])
        guard.consume({"type": "dahai", "actor": 1, "pai": "5p"})
        guard.consume({"type": "reach", "actor": 1})

        action = guard.guard_action(
            {
                "type": "dahai",
                "actor": 0,
                "pai": "5pr",
                "meta": meta_for_allowed((13, 0.1), (35, 0.9)),
            },
            [],
            "balanced",
        )

        self.assertEqual(action["pai"], "5p")
        self.assertEqual(action["meta"]["policy_guard"]["original_pai"], "5pr")


if __name__ == "__main__":
    unittest.main()
