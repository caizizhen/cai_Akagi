# `src/game_state` — Live game-state tracker

Wraps [`riichienv-core`](https://crates.io/crates/riichienv-core) to
maintain an authoritative, queryable mirror of the current mahjong game
fed by the MJAI event stream from the proxy bridge.

## Why a wrapper?

`riichienv-core` is great as a simulation engine but its public API is
shaped for RL training: `Vec<...>` everywhere, raw `u8` tile IDs in
0..136 with the red-five convention (16/52/88), `Phase` as a bare enum,
and so on. None of that is what a UI or downstream IPC layer wants.

This module:

1. Translates Akagi's own `schema::MjaiEvent` into the riichienv flavor
   (single field-level mismatch on `StartGame.id`, otherwise direct
   JSON round-trip — see `convert.rs`).
2. Drives `GameState::apply_mjai_event` with the converted events.
3. Provides a `GameStateSnapshot` whose tiles are mjai strings and
   whose enums use snake-case discriminants — straight to the wire.
4. Wraps the score / hand-evaluator helpers behind a stable interface
   so a riichienv API bump only touches this module.

## Files

| File          | Purpose                                              |
|---------------|------------------------------------------------------|
| `convert.rs`  | `to_riichienv(&AkagiEvent) -> Result<Option<RiEvent>>` |
| `tracker.rs`  | `GameTracker`, `spawn(rx) -> Arc<Mutex<GameTracker>>` |
| `snapshot.rs` | `GameStateSnapshot`, `PlayerSnapshot`, `MeldSnapshot` |
| `score.rs`    | `calculate_score`, `waits_for`, `is_tenpai`          |

## Wiring

Spawned from `lib.rs` once the MJAI bus exists:

```rust
let tracker = game_state::spawn(mjai_bus.subscribe());
// tracker: Arc<Mutex<GameTracker>>
//   → IPC commands (future) read snapshot via `tracker.lock().await.snapshot()`
```

The handle is held by `AppState` so future IPC commands can pull
snapshots without keeping a separate reference.

## Querying

```rust
let snap = tracker.lock().await.snapshot().expect("game in progress");
println!("oya: {}, dora: {:?}", snap.oya, snap.dora_markers);
```

`snapshot()` returns `None` until the first `start_game` event arrives.

## Score / wait helpers

These are pure functions; no state required:

```rust
use akagi::game_state::{calculate_score, waits_for};

// 3 han 30 fu, non-dealer ron, 0 honba.
let s = calculate_score(3, 30, false, false, 0);
assert_eq!(s.total, 3_900);

let waits = waits_for("123456789m123p1s")?;
assert_eq!(waits, vec!["1s"]);
```

The hand string is `riichienv`'s MPSZ notation, not mjai. Use
`(p123m)` etc. for melds.

## Adding a new event handler

`riichienv` already handles every protocol event in
`apply_mjai_event`. The only thing you'd add here is:

- A new field in `GameStateSnapshot` if the engine exposes something
  the UI needs but we're not surfacing yet (e.g. `last_win_results`).
- A patch in `convert.rs` if a new mjai event variant has a shape
  mismatch between Akagi and riichienv.

## Future: GameStateBus

When the IPC layer needs live state push (toast-style or reactive UI),
add a `GameStateBus` to `crate::event_bus` and have `tracker::run`
publish a snapshot on every event (or every N events). The current
design intentionally stops one step short of that — snapshot-on-pull
is enough for the read-only commands we need first.
