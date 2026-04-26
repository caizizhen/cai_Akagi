pub mod bot;
pub mod bridge;
pub mod cli;
pub mod config;
pub mod event_bus;
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
    let cfg = config::load_config(args.config.as_deref());

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

    // MJAI event bus is constructed unconditionally so future consumers
    // (HUD, storage, WS server) can subscribe without a config gate. The
    // BotManager subscribes only when bot.enabled is true.
    let mjai_bus = event_bus::mjai_bus();

    tauri::Builder::default()
        .setup(move |_app| {
            if cfg.bot.enabled {
                let bot_cfg = cfg.bot.clone();
                let mjai_for_bot = mjai_bus.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = spawn_bot_manager(bot_cfg, mjai_for_bot).await {
                        error!("Bot manager failed: {e:#}");
                    }
                });
            }

            if cfg.proxy.enabled {
                let proxy_cfg = cfg.proxy.clone();
                let platform = cfg.platform.kind;
                let session_for_proxy = session.clone();
                let mjai_for_proxy = Some(mjai_bus.clone());
                tauri::async_runtime::spawn(async move {
                    let shutdown = async {
                        let _ = tokio::signal::ctrl_c().await;
                    };
                    if let Err(e) = proxy::start_proxy(
                        proxy_cfg,
                        platform,
                        session_for_proxy,
                        mjai_for_proxy,
                        shutdown,
                    )
                    .await
                    {
                        error!("Proxy failed: {e}");
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn spawn_bot_manager(
    cfg: config::BotConfig,
    mjai: event_bus::MjaiBus,
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

    let response_bus = event_bus::bot_response_bus();
    let manager = bot::BotManager::new(runtime, registry, cfg.active, response_bus);
    let rx = mjai.subscribe();
    manager.run(rx).await
}
