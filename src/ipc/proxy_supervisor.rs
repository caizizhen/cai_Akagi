//! Lifecycle supervisor for the MITM proxy.
//!
//! The proxy itself (`crate::proxy::start_proxy`) is a long-running
//! future that takes a graceful-shutdown future. This module wraps it in
//! a control plane: a oneshot is the shutdown trigger, and a supervisor
//! task observes the proxy future's completion to flip
//! `ProxyStatus::Running → Stopped/Error` and clear the control handle.
//!
//! `ProxyStatus::Starting` is kept in the schema for future use but not
//! emitted today — bind happens inside hudsucker, so we have no
//! reliable "listening now" signal to gate Running on. We optimistically
//! flip to Running on spawn and let an Err on the join surface as Error
//! immediately if the bind in fact failed.

use crate::ipc::state::AppState;
use crate::proxy::start_proxy;
use crate::schema::ProxyStatus;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{Notify, oneshot};
use tracing::{error, info};

/// Spawn the proxy task and wire it into `state.proxy_control`. Returns
/// once the task has been spawned (not when the proxy is actually
/// listening).
pub async fn spawn_proxy_supervisor(state: AppState) -> Result<()> {
    let (proxy_cfg, platform) = {
        let cfg = state.config.read().await;
        (cfg.proxy.clone(), cfg.platform.kind)
    };
    let addr = proxy_cfg.addr.clone();

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    // Fresh Notify per spawn so old flows from a previous run can't be
    // re-kicked by a future stop. The handler holds a clone via start_proxy.
    let force_close: Arc<Notify> = Arc::new(Notify::new());

    {
        let mut ctl = state.proxy_control.lock().await;
        ctl.stop = Some(stop_tx);
        ctl.force_close = force_close.clone();
        ctl.status = ProxyStatus::Running { addr: addr.clone() };
    }
    let _ = state
        .proxy_status_bus
        .send(ProxyStatus::Running { addr: addr.clone() });

    let session = state.log_session.clone();
    let mjai = Some(state.mjai_bus.clone());
    let proxy_bus = state.proxy_status_bus.clone();
    let control = state.proxy_control.clone();

    tokio::spawn(async move {
        let shutdown = async move {
            let _ = stop_rx.await;
        };
        let result =
            start_proxy(proxy_cfg, platform, session, mjai, force_close, shutdown).await;

        let next = match &result {
            Ok(()) => {
                info!("proxy supervisor: stopped cleanly");
                ProxyStatus::Stopped
            }
            Err(e) => {
                error!("proxy supervisor: {e:#}");
                ProxyStatus::Error {
                    addr: Some(addr.clone()),
                    error: format!("{e:#}"),
                }
            }
        };

        {
            let mut ctl = control.lock().await;
            ctl.stop = None;
            ctl.status = next.clone();
        }
        let _ = proxy_bus.send(next);
    });

    Ok(())
}
