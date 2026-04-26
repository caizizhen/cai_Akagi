pub mod bridge;
pub mod cli;
pub mod config;
pub mod logger;
pub mod platform;
pub mod proxy;
pub mod schema;
pub mod util;

use clap::Parser;
use std::sync::Arc;
use tracing::{error, info};

pub fn run() {
    platform::setup();

    let args = cli::Cli::parse();
    let cfg = config::load_config(args.config.as_deref());

    let log_dir = util::resolve_dir(&cfg.logging.dir);
    let session = match logger::init(
        &log_dir,
        &cfg.logging.level,
        &cfg.logging.all_level,
        &[logger::LogTarget::new("proxy", "akagi::proxy")],
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("Failed to initialise logger: {e:?}");
            return;
        }
    };

    info!("Config loaded: {cfg:?}");
    info!("Log session at {}", session.dir().display());

    tauri::Builder::default()
        .setup(move |_app| {
            if cfg.proxy.enabled {
                let proxy_cfg = cfg.proxy.clone();
                let platform = cfg.platform.kind;
                let session_for_proxy = session.clone();
                tauri::async_runtime::spawn(async move {
                    let shutdown = async {
                        let _ = tokio::signal::ctrl_c().await;
                    };
                    if let Err(e) =
                        proxy::start_proxy(proxy_cfg, platform, session_for_proxy, shutdown).await
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
