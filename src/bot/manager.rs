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
//!
//! ## Status & notification emission
//!
//! Every lifecycle transition is published to two side-channel buses for
//! the IPC layer:
//!
//! - `BotStatusBus` — typed state machine
//!   (`Idle/Loading/Ready/Error/Stopped`). The frontend renders a spinner
//!   on `Loading{SyncingDeps}` so the user knows the slow first-run
//!   `uv sync` is in progress, not a hang.
//! - `NotifyBus` — toast-style notifications. Loading and error events
//!   reuse the same `id` (`"bot-loading-<name>"`) so the sticky
//!   "preparing" toast is replaced rather than duplicated when the spawn
//!   resolves.

use crate::bot::registry::BotRegistry;
use crate::bot::runner::{BotRunner, SubprocessBot};
use crate::bot::runtime::PythonRuntime;
use crate::event_bus::{BotResponseBus, BotStatusBus, NotifyBus};
use crate::schema::{BotStatus, LoadStage, MjaiEvent, Notification};
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
    status_tx: BotStatusBus,
    notify_tx: NotifyBus,
}

impl BotManager {
    pub fn new(
        runtime: PythonRuntime,
        registry: BotRegistry,
        active_name: String,
        out_tx: BotResponseBus,
        status_tx: BotStatusBus,
        notify_tx: NotifyBus,
    ) -> Self {
        Self {
            runtime,
            registry,
            active_name,
            runner: None,
            pending: Vec::new(),
            actor_id: None,
            out_tx,
            status_tx,
            notify_tx,
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
        // Surface the initial state to any IPC consumer that subscribes
        // late. Send is no-op when no subscribers exist yet.
        self.emit_status(BotStatus::Idle);
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
        let resp = match runner.react(&batch).await {
            Ok(r) => r,
            Err(e) => {
                let err_str = format!("{e:#}");
                let bot = self.active_name.clone();
                self.emit_status(BotStatus::Error {
                    bot: bot.clone(),
                    error: err_str.clone(),
                });
                self.emit_notify(
                    Notification::error("Bot reaction failed").body(err_str),
                );
                return Err(e).context("bot react failed");
            }
        };
        debug!(action = ?resp.action, "bot reacted");
        // MjaiEvent::None still goes on the bus — downstream consumers
        // decide whether to render. Centralizes the "skip" decision.
        let _ = self.out_tx.send(resp);

        if matches!(event, MjaiEvent::EndGame) {
            // Drain runner cleanly (writes end_game to stdin internally
            // through the next reset on the next start_game). Drop it
            // here so resources release immediately.
            let bot = self.active_name.clone();
            self.runner = None;
            self.actor_id = None;
            self.emit_status(BotStatus::Stopped { bot });
        }
        Ok(())
    }

    /// Two-phase spawn so the IPC layer can show a "Syncing deps…" spinner
    /// during the slow first-run path before the subprocess actually
    /// starts. Each branch publishes status + notification before
    /// returning so the UI never sees a stuck `Loading` state.
    async fn spawn_runner(&mut self) -> Result<()> {
        let bot_name = self.active_name.clone();
        let actor_id = self
            .actor_id
            .context("spawn_runner called without actor_id")?;

        let entry = match self.registry.find(&bot_name) {
            Some(e) => e.clone(),
            None => {
                let msg = format!(
                    "bot {:?} not found in registry at {}",
                    bot_name,
                    self.registry.root().display()
                );
                self.fail_load(&bot_name, &msg, "Bot not found");
                bail!(msg);
            }
        };
        if entry.pyproject.is_none() {
            let msg = format!(
                "bot {} has no pyproject.toml — required for uv sync",
                entry.name
            );
            self.fail_load(&bot_name, &msg, "Bot misconfigured");
            bail!(msg);
        }

        let load_id = format!("bot-loading-{bot_name}");

        // Phase 1: dep sync. ensure_synced is a no-op when stamp matches,
        // so the SyncingDeps state is brief on warm boots.
        self.emit_status(BotStatus::Loading {
            bot: bot_name.clone(),
            stage: LoadStage::SyncingDeps,
        });
        self.emit_notify(
            Notification::info("Preparing bot")
                .body("Installing Python dependencies — first launch may take a while.")
                .sticky()
                .id(load_id.clone()),
        );

        if let Err(e) = self.runtime.ensure_synced(&entry.dir).await {
            let msg = format!("uv sync failed: {e:#}");
            self.emit_status(BotStatus::Error {
                bot: bot_name.clone(),
                error: msg.clone(),
            });
            self.emit_notify(
                Notification::error("Bot dependency install failed")
                    .body(msg)
                    .id(load_id),
            );
            return Err(e).context("ensure_synced");
        }

        // Phase 2: subprocess spawn.
        self.emit_status(BotStatus::Loading {
            bot: bot_name.clone(),
            stage: LoadStage::Spawning,
        });

        let mut cmd = self.runtime.command_for(&entry.dir, &["bot.py"]);
        cmd.arg(actor_id.to_string());
        let bot = match SubprocessBot::spawn_with_command(
            cmd,
            self.runtime.clone(),
            &entry.dir,
            actor_id,
        )
        .await
        {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("subprocess spawn failed: {e:#}");
                self.emit_status(BotStatus::Error {
                    bot: bot_name.clone(),
                    error: msg.clone(),
                });
                self.emit_notify(
                    Notification::error("Bot subprocess failed to start")
                        .body(msg)
                        .id(load_id),
                );
                return Err(e);
            }
        };

        info!(bot = %bot_name, actor_id, "bot runner spawned");
        self.emit_status(BotStatus::Ready {
            bot: bot_name.clone(),
            actor_id,
        });
        // Reuse the loading id so the sticky toast is replaced, not
        // duplicated. Frontend treats same-id as a swap.
        self.emit_notify(
            Notification::success(format!("{bot_name} ready"))
                .id(load_id),
        );
        self.runner = Some(Box::new(bot));
        Ok(())
    }

    fn fail_load(&self, bot: &str, error: &str, title: &str) {
        self.emit_status(BotStatus::Error {
            bot: bot.into(),
            error: error.into(),
        });
        self.emit_notify(Notification::error(title.to_owned()).body(error.to_owned()));
    }

    fn emit_status(&self, s: BotStatus) {
        let _ = self.status_tx.send(s);
    }

    fn emit_notify(&self, n: Notification) {
        let _ = self.notify_tx.send(n);
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
    use crate::event_bus::{bot_response_bus, bot_status_bus, notify_bus};
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Records every `react` batch and replies with a scripted action.
    #[derive(Default)]
    struct MockBotRunner {
        calls: Arc<Mutex<Vec<Vec<MjaiEvent>>>>,
        next: Arc<Mutex<Vec<BotResponse>>>,
        /// If set, react() returns this error instead of consuming `next`.
        fail_with: Arc<Mutex<Option<String>>>,
    }

    impl MockBotRunner {
        fn new(replies: Vec<BotResponse>) -> (Self, Arc<Mutex<Vec<Vec<MjaiEvent>>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let r = Self {
                calls: calls.clone(),
                next: Arc::new(Mutex::new(replies)),
                fail_with: Arc::new(Mutex::new(None)),
            };
            (r, calls)
        }

        fn failing(err: &str) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                next: Arc::new(Mutex::new(Vec::new())),
                fail_with: Arc::new(Mutex::new(Some(err.into()))),
            }
        }
    }

    #[async_trait]
    impl BotRunner for MockBotRunner {
        async fn react(&mut self, events: &[MjaiEvent]) -> Result<BotResponse> {
            self.calls.lock().await.push(events.to_vec());
            if let Some(err) = self.fail_with.lock().await.as_deref() {
                bail!(err.to_string());
            }
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
    ) -> (
        BotManager,
        Arc<Mutex<Vec<Vec<MjaiEvent>>>>,
        broadcast::Receiver<BotResponse>,
        broadcast::Receiver<BotStatus>,
        broadcast::Receiver<Notification>,
    ) {
        let (mock, calls) = MockBotRunner::new(replies);
        let bus = bot_response_bus();
        let status = bot_status_bus();
        let notify = notify_bus();
        let resp_rx = bus.subscribe();
        let status_rx = status.subscribe();
        let notify_rx = notify.subscribe();
        let mut mgr = BotManager::new(
            dummy_runtime(),
            empty_registry(),
            "mock".into(),
            bus,
            status,
            notify,
        );
        // Pre-seat the actor and inject the mock so we don't go through
        // the registry / runtime path (covered by runner.rs tests).
        mgr.actor_id = Some(2);
        mgr.runner = Some(Box::new(mock));
        (mgr, calls, resp_rx, status_rx, notify_rx)
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
        let (mut mgr, calls, _, _, _) = manager_with_mock(vec![]);

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
        let (mut mgr, calls, _, _, _) = manager_with_mock(vec![]);
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
        let (mut mgr, calls, _, _, _) = manager_with_mock(vec![]);

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
        let (mut mgr, _, mut rx, _, _) = manager_with_mock(vec![scripted.clone()]);
        mgr.handle(dahai(0)).await.unwrap(); // others' dahai → flush

        let received = rx.try_recv().expect("bot response should be broadcast");
        assert_eq!(received, scripted);
    }

    #[tokio::test]
    async fn end_game_flushes_drops_runner_emits_stopped() {
        let (mut mgr, calls, _, mut status_rx, _) = manager_with_mock(vec![]);
        mgr.handle(MjaiEvent::EndGame).await.unwrap();
        assert_eq!(calls.lock().await.len(), 1);
        assert!(mgr.runner.is_none());
        assert!(mgr.actor_id.is_none());

        let status = status_rx.try_recv().expect("status emitted");
        assert!(
            matches!(status, BotStatus::Stopped { .. }),
            "expected Stopped, got {status:?}"
        );
    }

    #[tokio::test]
    async fn react_failure_emits_error_status_and_notification() {
        let bus = bot_response_bus();
        let status = bot_status_bus();
        let notify = notify_bus();
        let mut status_rx = status.subscribe();
        let mut notify_rx = notify.subscribe();
        let mut mgr = BotManager::new(
            dummy_runtime(),
            empty_registry(),
            "mock".into(),
            bus,
            status,
            notify,
        );
        mgr.actor_id = Some(2);
        mgr.runner = Some(Box::new(MockBotRunner::failing("kaboom")));

        // Trigger a decision point — react() returns error.
        let err = mgr.handle(dahai(0)).await.unwrap_err();
        assert!(format!("{err:#}").contains("react failed"));

        let s = status_rx.try_recv().unwrap();
        match s {
            BotStatus::Error { bot, error } => {
                assert_eq!(bot, "mock");
                assert!(error.contains("kaboom"), "got error: {error}");
            }
            other => panic!("expected Error, got {other:?}"),
        }

        let n = notify_rx.try_recv().unwrap();
        assert_eq!(n.level, crate::schema::NotifyLevel::Error);
        assert!(n.title.contains("Bot reaction failed"));
    }

    #[tokio::test]
    async fn missing_bot_in_registry_emits_error_status() {
        // Empty registry + handle(StartGame{id}) → spawn_runner errors,
        // emits BotStatus::Error and a notification.
        let bus = bot_response_bus();
        let status = bot_status_bus();
        let notify = notify_bus();
        let mut status_rx = status.subscribe();
        let mut notify_rx = notify.subscribe();
        let mut mgr = BotManager::new(
            dummy_runtime(),
            empty_registry(),
            "ghost".into(),
            bus,
            status,
            notify,
        );

        let err = mgr
            .handle(MjaiEvent::StartGame {
                names: ["a".into(), "b".into(), "c".into(), "d".into()],
                kyoku_first: None,
                aka_flag: None,
                id: Some(0),
            })
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("not found in registry"));

        let s = status_rx.try_recv().unwrap();
        assert!(
            matches!(s, BotStatus::Error { ref bot, .. } if bot == "ghost"),
            "expected Error{{bot=ghost}}, got {s:?}"
        );
        let n = notify_rx.try_recv().unwrap();
        assert_eq!(n.level, crate::schema::NotifyLevel::Error);
        assert!(n.title.contains("Bot not found"));
    }

    #[tokio::test]
    async fn events_before_start_game_are_dropped() {
        // Manager freshly constructed → no actor_id, no runner.
        let bus = bot_response_bus();
        let status = bot_status_bus();
        let notify = notify_bus();
        let mut mgr = BotManager::new(
            dummy_runtime(),
            empty_registry(),
            "mock".into(),
            bus,
            status,
            notify,
        );
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
        let status = bot_status_bus();
        let notify = notify_bus();
        let mut mgr = BotManager::new(
            dummy_runtime(),
            empty_registry(),
            "mock".into(),
            bot_bus,
            status,
            notify,
        );
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
