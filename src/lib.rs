pub mod bot;
pub mod bridge;
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
use tracing::{error, info, warn};

pub fn run() {
    platform::setup();

    let args = cli::Cli::parse();
    let (cfg, config_path) = config::load_config(args.config.as_deref());

    let log_dir = util::resolve_dir(&cfg.logging.dir);
    let session = match logger::init(
        &log_dir,
        &cfg.logging.level,
        &cfg.logging.all_level,
        &[
            logger::LogTarget::new("proxy", "akagi::proxy"),
            logger::LogTarget::new("bot", "akagi::bot"),
        ],
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("Failed to initialise logger: {e:?}");
            return;
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
    let proxy_status_bus = event_bus::proxy_status_bus();
    let notify_bus = event_bus::notify_bus();

    let bot_enabled = cfg.bot.enabled;
    let proxy_enabled = cfg.proxy.enabled;
    let bot_cfg = cfg.bot.clone();

    // Game-state tracker subscribes to the MJAI bus before AppState is
    // built so the Arc handle goes straight into state for future IPC
    // commands. The tracker task ends when all bus senders drop.
    let game_tracker = game_state::spawn(mjai_bus.subscribe());

    let state = ipc::AppState::new(
        cfg,
        config_path,
        session.clone(),
        mjai_bus.clone(),
        bot_response_bus.clone(),
        bot_status_bus.clone(),
        proxy_status_bus.clone(),
        notify_bus.clone(),
        game_tracker,
    );

    tauri::Builder::default()
        .invoke_handler(crate::ipc_handlers!())
        .setup({
            let state = state.clone();
            move |app| {
                ipc::install(&app.handle(), state.clone())?;

                if bot_enabled {
                    let mjai_for_bot = mjai_bus.clone();
                    let resp = bot_response_bus.clone();
                    let bs = bot_status_bus.clone();
                    let nb = notify_bus.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            spawn_bot_manager(bot_cfg, mjai_for_bot, resp, bs, nb).await
                        {
                            error!("Bot manager failed: {e:#}");
                        }
                    });
                }

                if proxy_enabled {
                    let state_for_proxy = state.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            ipc::proxy_supervisor::spawn_proxy_supervisor(state_for_proxy).await
                        {
                            error!("Proxy supervisor failed: {e:#}");
                        }
                    });
                }

                // Best-effort terminal ctrl_c → graceful proxy shutdown.
                // GUI close goes through Tauri's window event path, not
                // here, so this only matters when the user runs Akagi
                // headless from a terminal.
                let stop_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    let stop = {
                        let mut ctl = stop_state.proxy_control.lock().await;
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
) -> anyhow::Result<()> {
    let bot_dir = util::resolve_dir(std::path::Path::new(&cfg.dir));
    let registry = bot::BotRegistry::scan(&bot_dir)?;
    if registry.find(&cfg.active).is_none() {
        warn!(
            "configured bot {:?} not found under {}; available: {:?}",
            cfg.active,
            bot_dir.display(),
            registry.names().collect::<Vec<_>>()
        );
    }

    let runtime = bot::PythonRuntime::locate(None)?;
    info!(
        "bot runtime: python={} uv={} mode={:?}",
        runtime.python().display(),
        runtime.uv().display(),
        runtime.mode()
    );

    let manager = bot::BotManager::new(
        runtime,
        registry,
        cfg.active,
        response_bus,
        status_bus,
        notify_bus,
    );
    let rx = mjai.subscribe();
    manager.run(rx).await
}
