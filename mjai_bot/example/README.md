# Example Bot — Rule-based Shanten Optimizer

A first-party rule-based bot that ships with Akagi v3 to validate the
end-to-end pipeline (proxy → bridge → bus → bot manager → bot).

## How Akagi invokes this bot

- Working dir: this folder.
- Argv: `python bot.py <player_id>` (mjai.app convention).
- Env: `AKAGI_PLAYER_ID=<player_id>` (kept for parity).
- Stdin: one JSON array of `MjaiEvent`s per line.
- Stdout: one JSON object (mjai action) per line; `{"type":"none"}`
  when no action is owed.
- Stderr: free-form logs; Akagi pumps each line into `tracing` with
  `bot=example`.

`start_game.id` is treated as authoritative for seat assignment — argv
and env are just early hints that get overridden when the protocol kicks
in.

## Settings

`manifest.toml` exposes numeric thresholds (defence, fold, riichi, early
wall). Akagi writes the merged JSON to the path in `AKAGI_BOT_CONFIG` on
each `start_game`; see `src/bot/README.md` for the contract.

The bot also emits `meta.debug` on non-`none` actions. The debug payload
records the decision reason, wall count, live score pressure, safety tile
inventory, shanten, danger, ukeire, shape score, target yaku, and riichi /
damaten value signals when relevant. Use `scripts/evaluate_example_bot.py`
to replay saved mjai logs and compare two bot files by action and debug
reason.

`play_style` selects the high-level policy:

- `aggressive`: pushes value, riichi, calls, and kans while mostly ignoring
  deal-in risk.
- `balanced`: weighs deal-in risk, lost points, and win value together,
  but avoids riichi unless the hand is clearly worth pushing.
- `conservative`: treats defence as the first priority, avoids calls and
  riichi under pressure, and chooses safer discards before hand speed.
- `tanyao_fast`: races toward a fast open tanyao by shedding terminals /
  honors and calling simple tiles when that keeps or improves speed.

## Experience summary

At `end_game`, the bot appends a lightweight summary to `experience.json`
in this folder. The file stores aggregate counts and the last 20 game
summaries: decisions, riichi, calls, calls against riichi, dangerous
discards, wins, deal-ins, draws, and score delta. This is an experience
log for review and future tuning, not online model training.

Set `AKAGI_EXPERIENCE_PATH` to write the summary somewhere else.
