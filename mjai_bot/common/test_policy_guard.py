import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from policy_guard import ConservativePolicyGuard


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


if __name__ == "__main__":
    unittest.main()
