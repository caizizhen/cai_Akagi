//! Lifecycle + decision-point batching for the active bot.
//!
//! `BotManager` owns one `Box<dyn BotRunner>`, subscribes to the
//! `MjaiBus`, accumulates events between decision points, and broadcasts
//! every `BotResponse` (including `MjaiEvent::None`) onto a
//! `BotResponseBus` for downstream consumers (HUD, storage, external WS).
//!
//! Decision-point policy intentionally errs wide: any event where the
//! bot *might* be allowed to act flushes the pending batch. The bot
//! itself returns `MjaiEvent::None` when no action is owed, so we can't
//! be wrong about "allowed to act" — only about whether the round-trip
//! to the bot is worth the latency. Currently we accept that latency.

use crate::bot::registry::BotRegistry;
use crate::bot::runner::{BotRunner, SubprocessBot};
use crate::bot::runtime::PythonRuntime;
use crate::event_bus::BotResponseBus;
use crate::schema::MjaiEvent;
use anyhow::{Context, Result, bail};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

pub struct BotManager {
    runtime: PythonRuntime,
    registry: BotRegistry,
    /// Bot subdirectory name (matches a `BotEntry::name`).
    active_name: String,
    runner: Option<Box<dyn BotRunner>>,
    /// Events seen since the last `react()` call.
    pending: Vec<MjaiEvent>,
    /// Bot's seat in the current game; set on `start_game`.
    actor_id: Option<u8>,
    out_tx: BotResponseBus,
}

impl BotManager {
    pub fn new(
        runtime: PythonRuntime,
        registry: BotRegistry,
        active_name: String,
        out_tx: BotResponseBus,
    ) -> Self {
        Self {
            runtime,
            registry,
            active_name,
            runner: None,
            pending: Vec::new(),
            actor_id: None,
            out_tx,
        }
    }

    pub fn out_tx(&self) -> &BotResponseBus {
        &self.out_tx
    }

    /// Block on the MJAI receiver, dispatching every event through `handle`.
    /// Returns when the channel is closed (all senders dropped).
    ///
    /// Caller subscribes via `mjai.subscribe()` rather than passing the
    /// `Sender` so the manager doesn't keep the channel alive itself —
    /// makes shutdown deterministic when the proxy stops producing.
    pub async fn run(
        mut self,
        mut rx: broadcast::Receiver<MjaiEvent>,
    ) -> Result<()> {
        info!(
            bot = %self.active_name,
            "bot manager subscribed to MJAI bus; waiting for start_game"
        );
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Err(e) = self.handle(ev).await {
                        error!("bot manager: {e:#}");
                        // Tear the runner down; next start_game will respawn.
                        self.runner = None;
                        self.pending.clear();
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("bot manager lagged behind MJAI bus by {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("MJAI bus closed; bot manager exiting");
                    return Ok(());
                }
            }
        }
    }

    /// Drive one event through the manager. Public for unit tests.
    pub async fn handle(&mut self, event: MjaiEvent) -> Result<()> {
        // Spawn the runner the moment we see the bot's seat in start_game.
        if let MjaiEvent::StartGame { id: Some(seat), .. } = &event {
            self.actor_id = Some(*seat);
            self.spawn_runner().await?;
            self.pending.clear();
        }

        // No runner means we don't even have a seat yet (no start_game with
        // `id` seen). Drop the event silently — we have no one to feed.
        if self.runner.is_none() {
            return Ok(());
        }

        self.pending.push(event.clone());

        if !self.is_decision_point(&event) {
            return Ok(());
        }

        let runner = self
            .runner
            .as_mut()
            .expect("runner is Some — checked above");
        let batch = std::mem::take(&mut self.pending);
        let resp = runner.react(&batch).await.context("bot react failed")?;
        debug!(action = ?resp.action, "bot reacted");
        // MjaiEvent::None still goes on the bus — downstream consumers
        // decide whether to render. Centralizes the "skip" decision.
        let _ = self.out_tx.send(resp);

        if matches!(event, MjaiEvent::EndGame) {
            // Drain runner cleanly (writes end_game to stdin internally
            // through the next reset on the next start_game). Drop it
            // here so resources release immediately.
            self.runner = None;
            self.actor_id = None;
        }
        Ok(())
    }

    async fn spawn_runner(&mut self) -> Result<()> {
        let entry = self.registry.find(&self.active_name).with_context(|| {
            format!(
                "bot {:?} not found in registry at {}",
                self.active_name,
                self.registry.root().display()
            )
        })?;
        let actor_id = self
            .actor_id
            .context("spawn_runner called without actor_id")?;
        if entry.pyproject.is_none() {
            bail!(
                "bot {} has no pyproject.toml — required for uv sync",
                entry.name
            );
        }
        let bot = SubprocessBot::spawn(&self.runtime, &entry.dir, actor_id).await?;
        info!(bot = %entry.name, actor_id, "bot runner spawned");
        self.runner = Some(Box::new(bot));
        Ok(())
    }

    /// Conservative: every event that *might* be a decision point flushes
    /// the pending batch. Bot returns `MjaiEvent::None` when not its turn.
    fn is_decision_point(&self, e: &MjaiEvent) -> bool {
        let Some(me) = self.actor_id else {
            return false;
        };
        match e {
            // Own draws — bot decides discard / riichi / agari / kan.
            MjaiEvent::Tsumo { actor, .. } => *actor == me,
            // Others' calls / discards may open a chi/pon/kan/ron window.
            MjaiEvent::Dahai { actor, .. } => *actor != me,
            MjaiEvent::Kakan { actor, .. } => *actor != me,
            // Round / game boundaries: bot may want to flush state.
            MjaiEvent::ReachAccepted { .. }
            | MjaiEvent::Hora { .. }
            | MjaiEvent::Ryukyoku { .. }
            | MjaiEvent::EndKyoku
            | MjaiEvent::EndGame => true,
            // Everything else (start_game/start_kyoku, our own dahai,
            // chi/pon/daiminkan/ankan, dora reveal, reach declaration)
            // accumulates without bothering the bot — its state catches
            // up the next time we flush.
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::types::BotResponse;
    use crate::event_bus::bot_response_bus;
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Records every `react` batch and replies with a scripted action.
    #[derive(Default)]
    struct MockBotRunner {
        calls: Arc<Mutex<Vec<Vec<MjaiEvent>>>>,
        next: Arc<Mutex<Vec<BotResponse>>>,
    }

    impl MockBotRunner {
        fn new(replies: Vec<BotResponse>) -> (Self, Arc<Mutex<Vec<Vec<MjaiEvent>>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let r = Self {
                calls: calls.clone(),
                next: Arc::new(Mutex::new(replies)),
            };
            (r, calls)
        }
    }

    #[async_trait]
    impl BotRunner for MockBotRunner {
        async fn react(&mut self, events: &[MjaiEvent]) -> Result<BotResponse> {
            self.calls.lock().await.push(events.to_vec());
            let mut q = self.next.lock().await;
            if q.is_empty() {
                Ok(BotResponse {
                    action: MjaiEvent::None,
                    meta: None,
                })
            } else {
                Ok(q.remove(0))
            }
        }
        async fn reset(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn dummy_runtime() -> PythonRuntime {
        PythonRuntime::from_paths(
            PathBuf::from("/dev/null/python"),
            PathBuf::from("/dev/null/uv"),
            crate::bot::runtime::RuntimeMode::System,
        )
    }

    fn empty_registry() -> BotRegistry {
        BotRegistry::default()
    }

    fn manager_with_mock(
        replies: Vec<BotResponse>,
    ) -> (BotManager, Arc<Mutex<Vec<Vec<MjaiEvent>>>>, broadcast::Receiver<BotResponse>) {
        let (mock, calls) = MockBotRunner::new(replies);
        let bus = bot_response_bus();
        let rx = bus.subscribe();
        let mut mgr = BotManager::new(dummy_runtime(), empty_registry(), "mock".into(), bus);
        // Pre-seat the actor and inject the mock so we don't go through
        // the registry / runtime path (covered by runner.rs tests).
        mgr.actor_id = Some(2);
        mgr.runner = Some(Box::new(mock));
        (mgr, calls, rx)
    }

    fn dahai(actor: u8) -> MjaiEvent {
        MjaiEvent::Dahai {
            actor,
            pai: "1m".into(),
            tsumogiri: false,
        }
    }

    #[tokio::test]
    async fn non_decision_events_accumulate_without_calling_react() {
        let (mut mgr, calls, _rx) = manager_with_mock(vec![]);

        // None of these are decision points for seat 2:
        //   - own dahai (seat 2)
        //   - own tsumo
        //   - dora reveal
        mgr.handle(MjaiEvent::Tsumo {
            actor: 0,
            pai: "1m".into(),
        })
        .await
        .unwrap(); // not our tsumo, but is also NOT in decision set
        mgr.handle(dahai(2)).await.unwrap(); // our own dahai
        mgr.handle(MjaiEvent::Dora {
            dora_marker: "5p".into(),
        })
        .await
        .unwrap();

        assert!(
            calls.lock().await.is_empty(),
            "no decision points → no react calls"
        );
        assert_eq!(mgr.pending.len(), 3, "events should be buffered");
    }

    #[tokio::test]
    async fn others_dahai_flushes_batch() {
        let (mut mgr, calls, _rx) = manager_with_mock(vec![]);
        mgr.handle(MjaiEvent::Dora {
            dora_marker: "5p".into(),
        })
        .await
        .unwrap();
        mgr.handle(dahai(0)).await.unwrap(); // someone else's dahai

        let calls = calls.lock().await;
        assert_eq!(calls.len(), 1, "exactly one react call");
        assert_eq!(calls[0].len(), 2, "batch carries the buffered + trigger");
        assert!(matches!(calls[0][0], MjaiEvent::Dora { .. }));
        assert!(matches!(calls[0][1], MjaiEvent::Dahai { actor: 0, .. }));
    }

    #[tokio::test]
    async fn own_tsumo_flushes_others_tsumo_does_not() {
        let (mut mgr, calls, _rx) = manager_with_mock(vec![]);

        // Others' tsumo: NOT a decision point.
        mgr.handle(MjaiEvent::Tsumo {
            actor: 0,
            pai: "1m".into(),
        })
        .await
        .unwrap();
        assert!(calls.lock().await.is_empty());

        // Our tsumo: IS a decision point.
        mgr.handle(MjaiEvent::Tsumo {
            actor: 2,
            pai: "5m".into(),
        })
        .await
        .unwrap();
        let calls = calls.lock().await;
        assert_eq!(calls.len(), 1);
        // Both events flushed in the batch.
        assert_eq!(calls[0].len(), 2);
    }

    #[tokio::test]
    async fn bot_response_broadcast_to_subscribers() {
        let scripted = BotResponse {
            action: dahai(2),
            meta: None,
        };
        let (mut mgr, _, mut rx) = manager_with_mock(vec![scripted.clone()]);
        mgr.handle(dahai(0)).await.unwrap(); // others' dahai → flush

        let received = rx.try_recv().expect("bot response should be broadcast");
        assert_eq!(received, scripted);
    }

    #[tokio::test]
    async fn end_game_flushes_drops_runner_clears_seat() {
        let (mut mgr, calls, _rx) = manager_with_mock(vec![]);
        mgr.handle(MjaiEvent::EndGame).await.unwrap();
        assert_eq!(calls.lock().await.len(), 1);
        assert!(mgr.runner.is_none());
        assert!(mgr.actor_id.is_none());
    }

    #[tokio::test]
    async fn events_before_start_game_are_dropped() {
        // Manager freshly constructed → no actor_id, no runner.
        let bus = bot_response_bus();
        let mut mgr = BotManager::new(dummy_runtime(), empty_registry(), "mock".into(), bus);
        // Should not panic / error even with no runner.
        mgr.handle(dahai(0)).await.unwrap();
        assert!(mgr.pending.is_empty());
    }

    #[tokio::test]
    async fn run_returns_ok_when_bus_closes() {
        // Subscribe outside the task so the task holds only the Receiver.
        // Dropping the Sender outside causes a clean Closed → Ok(()) exit.
        let mjai = crate::event_bus::mjai_bus();
        let rx = mjai.subscribe();

        let bot_bus = bot_response_bus();
        let mut mgr =
            BotManager::new(dummy_runtime(), empty_registry(), "mock".into(), bot_bus);
        mgr.actor_id = Some(2);
        let (mock, _calls) = MockBotRunner::new(vec![]);
        mgr.runner = Some(Box::new(mock));

        let handle = tokio::spawn(async move { mgr.run(rx).await });
        drop(mjai); // last sender → channel closes
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("manager exited")
            .expect("join")
            .expect("Ok");
    }
}
