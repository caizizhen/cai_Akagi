pub mod analysis;
pub mod bot;
pub mod bridge;
pub mod capture;
pub mod cli;
pub mod config;
pub mod event_bus;
pub mod game_state;
pub mod ipc;
pub mod logger;
pub mod platform;
pub mod proxy;
pub mod schema;
pub mod util;

use clap::Parser;
use std::sync::Arc;
use tauri::Manager;
use tracing::{error, info, warn};

pub fn run() {
    platform::setup();

    let args = cli::Cli::parse();
    let (cfg, config_path) = config::load_config(args.config.as_deref());

    let log_dir = util::resolve_dir(&cfg.logging.dir);
    let targets = [
        logger::LogTarget::new("proxy", "akagi::proxy"),
        logger::LogTarget::new("bot", "akagi::bot"),
    ];
    let session = match logger::init(
        &log_dir,
        &cfg.logging.level,
        &cfg.logging.all_level,
        &targets,
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            // Read-only fs fallback: retry under user data dir.
            if let Some(fallback) =
                util::user_config_root().map(|r| r.join(util::strip_leading_dot(&cfg.logging.dir)))
            {
                if fallback != log_dir {
                    eprintln!(
                        "Logger init at {} failed: {e:?}. Retrying at {}",
                        log_dir.display(),
                        fallback.display()
                    );
                    match logger::init(
                        &fallback,
                        &cfg.logging.level,
                        &cfg.logging.all_level,
                        &targets,
                    ) {
                        Ok(s) => Arc::new(s),
                        Err(e2) => {
                            eprintln!("Failed to initialise logger: {e2:?}");
                            return;
                        }
                    }
                } else {
                    eprintln!("Failed to initialise logger: {e:?}");
                    return;
                }
            } else {
                eprintln!("Failed to initialise logger: {e:?}");
                return;
            }
        }
    };

    info!("Config loaded: {cfg:?}");
    info!("Log session at {}", session.dir().display());

    // All buses constructed up front so AppState owns the canonical
    // sender clones — every subsystem (bot manager, proxy supervisor,
    // ipc forwarders, future HUD) shares these handles via state.
    let mjai_bus = event_bus::mjai_bus();
    let bot_response_bus = event_bus::bot_response_bus();
    let bot_status_bus = event_bus::bot_status_bus();
    let capture_status_bus = event_bus::capture_status_bus();
    let notify_bus = event_bus::notify_bus();
    let analysis_bus = event_bus::analysis_bus();
    let post_tracker_bus = event_bus::post_tracker_bus();

    let bot_enabled = cfg.bot.enabled;
    let proxy_enabled = cfg.proxy.enabled;
    let bot_cfg = cfg.bot.clone();

    // Game-state tracker handle is built up front so AppState can carry
    // the Arc, but the consumer task is spawned inside `.setup()` once
    // the Tauri Tokio runtime is live (sync `lib::run` has no runtime).
    let game_tracker = game_state::tracker::new_handle();
    let analysis_cache = std::sync::Arc::new(tokio::sync::RwLock::new(None));

    let tracker_rx = mjai_bus.subscribe();
    let tracker_post = post_tracker_bus.clone();
    let analysis_rx = post_tracker_bus.subscribe();
    let analysis_tracker = game_tracker.clone();
    let analysis_bus_for_runner = analysis_bus.clone();
    let analysis_cache_for_runner = analysis_cache.clone();

    tauri::Builder::default()
        .invoke_handler(crate::ipc_handlers!())
        .setup({
            // AppState constructed *inside* setup() so the python+uv
            // runtime can be located using `resource_dir` — bundled
            // binaries live under `<resource_dir>/runtime/...` and that
            // path is only resolvable once the AppHandle exists.
            move |app| {
                let resource_dir = app.path().resource_dir().ok();
                let runtime = bot::PythonRuntime::locate(resource_dir.as_deref()).ok();
                match &runtime {
                    Some(rt) => info!(
                        "bot runtime: python={} uv={} mode={:?}",
                        rt.python().display(),
                        rt.uv().display(),
                        rt.mode()
                    ),
                    None => warn!(
                        "no python3+uv runtime found (neither bundled nor on PATH); bot install/sync will be unavailable"
                    ),
                }

                let state = ipc::AppState::new(
                    cfg,
                    config_path,
                    session.clone(),
                    mjai_bus.clone(),
                    bot_response_bus.clone(),
                    bot_status_bus.clone(),
                    capture_status_bus.clone(),
                    notify_bus.clone(),
                    analysis_bus.clone(),
                    game_tracker,
                    analysis_cache,
                    runtime.clone(),
                );

                ipc::install(&app.handle(), state.clone())?;

                // Spawn tracker + analysis loops inside the Tauri Tokio
                // runtime — `lib::run` itself is sync.
                tauri::async_runtime::spawn(game_state::tracker::drive_loop(
                    state.game_tracker.clone(),
                    tracker_rx,
                    Some(tracker_post),
                ));
                tauri::async_runtime::spawn(analysis::runner::drive_loop(
                    analysis_rx,
                    analysis_tracker,
                    analysis_bus_for_runner,
                    analysis_cache_for_runner,
                ));

                if bot_enabled {
                    let mjai_for_bot = mjai_bus.clone();
                    let resp = bot_response_bus.clone();
                    let bs = bot_status_bus.clone();
                    let nb = notify_bus.clone();
                    let rt = runtime.clone();
                    let syncs = state.syncs_in_flight.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            spawn_bot_manager(bot_cfg, mjai_for_bot, resp, bs, nb, rt, syncs)
                                .await
                        {
                            error!("Bot manager failed: {e:#}");
                        }
                    });
                }

                if proxy_enabled {
                    let state_for_capture = state.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            ipc::capture_supervisor::spawn_capture_supervisor(state_for_capture)
                                .await
                        {
                            error!("Capture supervisor failed: {e:#}");
                        }
                    });
                }

                // Best-effort terminal ctrl_c → graceful capture shutdown.
                // GUI close goes through Tauri's window event path, not
                // here, so this only matters when the user runs Akagi
                // headless from a terminal.
                let stop_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    let stop = {
                        let mut ctl = stop_state.capture_control.lock().await;
                        ctl.stop.take()
                    };
                    if let Some(tx) = stop {
                        let _ = tx.send(());
                    }
                });

                Ok(())
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn spawn_bot_manager(
    cfg: config::BotConfig,
    mjai: event_bus::MjaiBus,
    response_bus: event_bus::BotResponseBus,
    status_bus: event_bus::BotStatusBus,
    notify_bus: event_bus::NotifyBus,
    runtime: Option<bot::PythonRuntime>,
    syncs_in_flight: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
) -> anyhow::Result<()> {
    let bot_dir = util::resolve_dir(std::path::Path::new(&cfg.dir));
    let registry = bot::BotRegistry::scan(&bot_dir)?;
    for (label, name) in [("4p", &cfg.active_4p), ("3p", &cfg.active_3p)] {
        if !name.is_empty() && registry.find(name).is_none() {
            warn!(
                "configured {} bot {:?} not found under {}; available: {:?}",
                label,
                name,
                bot_dir.display(),
                registry.names().collect::<Vec<_>>()
            );
        }
    }

    let runtime = runtime.ok_or_else(|| {
        anyhow::anyhow!("bot mode is enabled but no python3+uv runtime was found on PATH")
    })?;

    let manager = bot::BotManager::new(
        runtime,
        registry,
        cfg.active_4p,
        cfg.active_3p,
        response_bus,
        status_bus,
        notify_bus,
        syncs_in_flight,
    );
    let rx = mjai.subscribe();
    manager.run(rx).await
}
