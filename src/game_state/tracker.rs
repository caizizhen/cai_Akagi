//! `GameTracker` — observer-mode wrapper around `riichienv_core::state::GameState`.
//!
//! Subscribes to [`crate::event_bus::MjaiBus`], translates each
//! `schema::MjaiEvent` into a `riichienv` event, and feeds it through
//! `apply_mjai_event` so the engine maintains a live snapshot of the
//! game (hands, melds, river, scores, dora indicators, phase).
//!
//! # Lifecycle
//!
//! - On the first `StartGame`, a fresh `GameState` is constructed (the
//!   constructor calls `_initialize_round(0, round_wind=0, 0, 0, ...)`,
//!   so we get East 1 set up by default).
//! - On every subsequent `StartGame`, we drop and reconstruct — full
//!   reset, since `apply_mjai_event(StartGame)` only clears legal-action
//!   stale state and not scores/honba.
//! - All other events go through `apply_mjai_event`, which handles
//!   `StartKyoku` (round reset), tile draws/discards, melds, and round
//!   end.
//! - `MjaiEvent::None` (Akagi-only sentinel for bot replies) is skipped
//!   silently in `convert::to_riichienv`.
//!
//! # Concurrent access
//!
//! `spawn` returns an `Arc<Mutex<GameTracker>>` so future IPC commands
//! can pull a snapshot without going through a separate bus. The IPC
//! layer is intentionally not wired in this round — the tracker is
//! ready to be exposed when the frontend needs it.

use crate::game_state::convert;
use crate::game_state::snapshot::GameStateSnapshot;
use crate::schema::MjaiEvent as AkagiEvent;
use anyhow::Result;
use riichienv_core::rule::GameRule;
use riichienv_core::state::GameState;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{info, warn};

pub struct GameTracker {
    state: Option<GameState>,
    rule: GameRule,
    /// Total events fed since process start. Useful for "is the bridge
    /// alive?" checks; not reset on game boundaries.
    pub events_seen: u64,
}

impl GameTracker {
    pub fn new() -> Self {
        Self {
            state: None,
            rule: GameRule::default_tenhou(),
            events_seen: 0,
        }
    }

    /// Drive one event through the engine. `Ok(())` even when the event
    /// is a no-op (e.g. `MjaiEvent::None`) — the only error path is a
    /// JSON conversion failure, which means a malformed event.
    pub fn handle(&mut self, ev: &AkagiEvent) -> Result<()> {
        self.events_seen += 1;

        if matches!(ev, AkagiEvent::StartGame { .. }) {
            // Fresh game → fresh GameState. Constructor seeds round 0
            // (E-1) with default scores per `mode.starting_score()`.
            self.state = Some(GameState::new(0, true, None, 0, self.rule.clone()));
        }

        let Some(ri) = convert::to_riichienv(ev)? else {
            return Ok(()); // Skipped (e.g. MjaiEvent::None).
        };

        if let Some(s) = self.state.as_mut() {
            s.apply_mjai_event(ri);
        }
        Ok(())
    }

    /// Snapshot of the current state. Returns `None` if no game has
    /// started yet.
    pub fn snapshot(&self) -> Option<GameStateSnapshot> {
        self.state.as_ref().map(GameStateSnapshot::from_state)
    }

    /// Borrow the live engine state. For advanced use cases (e.g. running
    /// `HandEvaluator` against the observer's hand). Most callers should
    /// prefer `snapshot()` so the wire shape stays decoupled.
    pub fn state(&self) -> Option<&GameState> {
        self.state.as_ref()
    }
}

impl Default for GameTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a tracker task that consumes the given MJAI receiver. Returns
/// a shared handle for snapshot access.
///
/// The task ends cleanly when the broadcast channel closes (all
/// `MjaiBus` senders dropped).
pub fn spawn(rx: broadcast::Receiver<AkagiEvent>) -> Arc<Mutex<GameTracker>> {
    let tracker = Arc::new(Mutex::new(GameTracker::new()));
    let cloned = tracker.clone();
    tokio::spawn(async move { run(cloned, rx).await });
    tracker
}

async fn run(tracker: Arc<Mutex<GameTracker>>, mut rx: broadcast::Receiver<AkagiEvent>) {
    info!("game tracker subscribed to MJAI bus");
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let mut t = tracker.lock().await;
                if let Err(e) = t.handle(&ev) {
                    warn!("game tracker: handle error: {e:#}");
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("game tracker lagged behind MJAI bus by {n} events");
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!("MJAI bus closed; game tracker exiting");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::mjai_bus;

    fn start_game() -> AkagiEvent {
        AkagiEvent::StartGame {
            names: ["a".into(), "b".into(), "c".into(), "d".into()],
            kyoku_first: None,
            aka_flag: None,
            id: Some(0),
        }
    }

    fn start_kyoku(oya: u8) -> AkagiEvent {
        // 13 tiles per hand, garbage-but-parseable.
        let one_hand: [String; 13] = std::array::from_fn(|_| "1m".into());
        AkagiEvent::StartKyoku {
            bakaze: "E".into(),
            dora_marker: "2m".into(),
            kyoku: 1,
            honba: 0,
            kyotaku: 0,
            oya,
            scores: [25_000, 25_000, 25_000, 25_000],
            tehais: std::array::from_fn(|_| one_hand.clone()),
        }
    }

    #[test]
    fn tracker_starts_empty() {
        let t = GameTracker::new();
        assert!(t.snapshot().is_none());
        assert!(t.state().is_none());
    }

    #[test]
    fn start_game_constructs_state() {
        let mut t = GameTracker::new();
        t.handle(&start_game()).unwrap();
        let snap = t.snapshot().expect("snapshot after start_game");
        assert_eq!(snap.players.len(), 4);
        assert_eq!(snap.bakaze, "E");
        // Constructor seeded each player with the rule's starting score.
        for p in &snap.players {
            assert!(p.score > 0);
        }
    }

    #[test]
    fn start_kyoku_propagates_oya_and_scores() {
        let mut t = GameTracker::new();
        t.handle(&start_game()).unwrap();
        t.handle(&start_kyoku(2)).unwrap();
        let snap = t.snapshot().unwrap();
        assert_eq!(snap.oya, 2);
        assert_eq!(snap.honba, 0);
        for p in &snap.players {
            assert_eq!(p.score, 25_000);
        }
    }

    #[test]
    fn none_event_is_skipped() {
        let mut t = GameTracker::new();
        // No state yet → handle(None) shouldn't panic or construct anything.
        t.handle(&AkagiEvent::None).unwrap();
        assert!(t.state().is_none());
    }

    #[test]
    fn second_start_game_resets_state() {
        let mut t = GameTracker::new();
        t.handle(&start_game()).unwrap();
        t.handle(&start_kyoku(3)).unwrap();
        let first = t.snapshot().unwrap();
        assert_eq!(first.oya, 3);

        // New game with default oya=0 from constructor.
        t.handle(&start_game()).unwrap();
        let second = t.snapshot().unwrap();
        assert_eq!(second.oya, 0, "fresh state defaults to oya=0");
    }

    #[test]
    fn events_seen_counter_advances() {
        let mut t = GameTracker::new();
        t.handle(&start_game()).unwrap();
        t.handle(&start_kyoku(0)).unwrap();
        t.handle(&AkagiEvent::None).unwrap();
        assert_eq!(t.events_seen, 3);
    }

    #[tokio::test]
    async fn spawn_consumes_bus_until_closed() {
        let bus = mjai_bus();
        let rx = bus.subscribe();
        let tracker = spawn(rx);

        bus.send(start_game()).unwrap();
        bus.send(start_kyoku(1)).unwrap();

        // Give the task a moment to drain.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let snap = tracker.lock().await.snapshot().expect("snapshot");
        assert_eq!(snap.oya, 1);

        // Drop the last sender → channel closes → task exits cleanly.
        drop(bus);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
