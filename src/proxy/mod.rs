mod ca;
mod handler;

pub use handler::ProxyHandler;

use crate::{
    config::{Platform, ProxyConfig},
    logger::Session,
    util::resolve_dir,
};
use anyhow::{Context, Result};
use hudsucker::{Proxy, rustls};
use std::{future::Future, net::SocketAddr, str::FromStr, sync::Arc};
use tracing::info;

pub async fn start_proxy<F>(
    config: ProxyConfig,
    platform: Platform,
    session: Arc<Session>,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let ca_dir = resolve_dir(&config.ca_dir);
    info!("Using CA dir: {}", ca_dir.display());

    let ca = ca::load_or_generate(&ca_dir)?;
    let addr = SocketAddr::from_str(&config.addr)
        .with_context(|| format!("Invalid proxy addr: {}", config.addr))?;

    let handler = ProxyHandler::new(session.clone(), platform)?;

    info!("Starting proxy on {addr}");

    let proxy = Proxy::builder()
        .with_addr(addr)
        .with_ca(ca)
        .with_rustls_connector(rustls::crypto::aws_lc_rs::default_provider())
        .with_http_handler(handler.clone())
        .with_websocket_handler(handler)
        .with_graceful_shutdown(shutdown)
        .build()
        .context("Failed to build proxy")?;

    proxy.start().await.context("Proxy stopped with error")
}
