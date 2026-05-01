//! Persisted game history (`history/index.jsonl` + `history/games/*.mjai.jsonl`).
//!
//! Wired in `lib.rs`: a single `recorder::drive_loop` subscribes to the
//! `MjaiBus` and finalises each `EndGame`-terminated stream by:
//!
//! 1. running [`aggregator::aggregate`] over the buffered events to produce
//!    a `crate::schema::GameRecord` (final scores via the Mortal-style
//!    100k normalisation, ranks via stable score-desc/seat-asc tiebreak,
//!    plus per-game stat counters mirroring `libriichi/src/stat.rs`),
//! 2. writing the full event stream as `games/<id>.mjai.jsonl`,
//! 3. appending the record JSON line to `index.jsonl`,
//! 4. fan-out via `HistoryBus` so the IPC forwarder can emit
//!    `history-recorded` to the frontend.
//!
//! Mid-game disconnects are handled by *not* finalising — a buffer that
//! never sees `EndGame` is dropped on the next `StartGame` or on
//! shutdown. The Majsoul bridge only emits `EndGame` upon receiving the
//! server's `NotifyGameEndResult`, so disconnect-survivable behaviour is
//! "correct by construction".
//!
//! See `src/history/README.md` for module-extension guidance.

pub mod aggregator;
pub mod recorder;
pub mod store;

pub use store::HistoryStore;
