//! Shared state held by Tauri as `tauri::State<AppState>`.
//!
//! `AppState` owns the long-lived handles every IPC command and forwarder
//! needs: a clone of each `event_bus` `Sender`, the live `AppConfig`, the
//! log session directory, and the runtime handle that lets commands
//! start / stop the proxy on demand.
//!
//! Snapshots vs streams: the `*_bus` channels are the canonical event
//! stream, but a frontend that opens a window mid-game needs a one-shot
//! "what's the current state?" answer. To serve that, the IPC forwarder
//! task mirrors every status event into the `bot_status` /
//! `proxy_control.status` slots, and the `get_status` command reads them
//! back. Commands that *change* state (set_active_bot, start/stop_proxy)
//! also write the snapshot synchronously so a follow-up `get_status` is
//! always consistent with the action that just succeeded.

use crate::analysis::runner::AnalysisCache;
use crate::config::AppConfig;
use crate::event_bus::{
    AnalysisBus, BotResponseBus, BotStatusBus, MjaiBus, NotifyBus, ProxyStatusBus,
};
use crate::game_state::GameTracker;
use crate::logger::Session;
use crate::schema::{BotStatus, ProxyStatus};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, RwLock, oneshot};

/// Per-running-proxy control handle.
///
/// `stop` is `Some` while the proxy task is alive; sending `()` triggers
/// `with_graceful_shutdown` and the task exits. After exit the
/// supervisor sets `stop = None` and updates `status`.
///
/// `force_close` is shared with the active `ProxyHandler`; calling
/// `notify_waiters()` kicks every in-flight WebSocket flow so the game
/// client actually disconnects (graceful shutdown alone only blocks new
/// connections — existing flows would otherwise drain naturally).
pub struct ProxyControl {
    pub status: ProxyStatus,
    pub stop: Option<oneshot::Sender<()>>,
    pub force_close: Arc<Notify>,
}

impl Default for ProxyControl {
    fn default() -> Self {
        Self {
            status: ProxyStatus::Stopped,
            stop: None,
            force_close: Arc::new(Notify::new()),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub config_path: Arc<PathBuf>,
    pub log_session: Arc<Session>,

    pub mjai_bus: MjaiBus,
    pub bot_response_bus: BotResponseBus,
    pub bot_status_bus: BotStatusBus,
    pub proxy_status_bus: ProxyStatusBus,
    pub notify_bus: NotifyBus,
    pub analysis_bus: AnalysisBus,

    /// Latest BotStatus seen on the bus. Forwarder writes; commands read.
    pub bot_status: Arc<RwLock<BotStatus>>,
    pub proxy_control: Arc<Mutex<ProxyControl>>,
    /// Live game-state mirror. Future IPC commands lock this and call
    /// `snapshot()` to expose hands/scores/dora to the frontend.
    pub game_tracker: Arc<Mutex<GameTracker>>,
    /// Latest analysis result, populated by the analysis runner. Read by
    /// the `get_analysis` Tauri command for one-shot queries.
    pub analysis_cache: AnalysisCache,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AppConfig,
        config_path: PathBuf,
        log_session: Arc<Session>,
        mjai_bus: MjaiBus,
        bot_response_bus: BotResponseBus,
        bot_status_bus: BotStatusBus,
        proxy_status_bus: ProxyStatusBus,
        notify_bus: NotifyBus,
        analysis_bus: AnalysisBus,
        game_tracker: Arc<Mutex<GameTracker>>,
        analysis_cache: AnalysisCache,
    ) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path: Arc::new(config_path),
            log_session,
            mjai_bus,
            bot_response_bus,
            bot_status_bus,
            proxy_status_bus,
            notify_bus,
            analysis_bus,
            bot_status: Arc::new(RwLock::new(BotStatus::Idle)),
            proxy_control: Arc::new(Mutex::new(ProxyControl::default())),
            game_tracker,
            analysis_cache,
        }
    }
}
