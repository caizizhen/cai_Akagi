//! Lifecycle supervisor for the active capture backend.
//!
//! One supervisor instance multiplexes the two backends (`HudsuckerBackend`,
//! `ChromiumBackend`) — the one that runs is determined by
//! `cfg.capture.mode`. The control-plane shape (start, stop, status
//! reporting) is the same across both, so we reuse the existing
//! `AppState.proxy_control` and `AppState.proxy_status_bus` for now.
//! Phase 3 will rename these to `capture_control` / `capture_status_bus`.

use crate::capture::{
    CaptureBackend, CaptureCtx, CaptureKind, ShutdownToken, chromium::ChromiumBackend,
    hudsucker_backend::HudsuckerBackend,
};
use crate::config::CaptureMode;
use crate::ipc::state::AppState;
use crate::schema::ProxyStatus;
use anyhow::Result;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Stop the running backend (if any) and wait briefly for the
/// supervisor task to flip status to `Stopped` before returning. Used by
/// `restart_capture` so the spawn that follows starts on a clean slate.
async fn stop_and_wait(state: &AppState, max_wait: Duration) {
    // Subscribe *before* signalling shutdown so we can't lose the
    // resulting status emission to a race.
    let mut rx = state.proxy_status_bus.subscribe();

    let (stop, force_close) = {
        let mut ctl = state.proxy_control.lock().await;
        (ctl.stop.take(), ctl.force_close.clone())
    };
    let Some(tx) = stop else {
        return; // not running
    };
    // Kick in-flight WS flows so existing connections actually disconnect
    // (mirrors stop_proxy semantics).
    force_close.notify_waiters();
    let _ = tx.send(());

    let _ = tokio::time::timeout(max_wait, async {
        loop {
            match rx.recv().await {
                Ok(ProxyStatus::Stopped) | Ok(ProxyStatus::Error { .. }) => break,
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    })
    .await;
}

/// Stop the running backend (if any) and start a fresh one based on the
/// current `cfg.capture.mode`. Idempotent: safe to call when nothing is
/// running (becomes a plain `spawn`).
pub async fn restart_capture(state: AppState) -> Result<()> {
    stop_and_wait(&state, Duration::from_secs(2)).await;
    spawn_capture_supervisor(state).await
}

/// Spawn the capture supervisor task and wire it into
/// `state.proxy_control`. Returns once the task has been spawned (not
/// when the backend is actually live).
pub async fn spawn_capture_supervisor(state: AppState) -> Result<()> {
    // Defensive: if a previous task is still alive, refuse rather than
    // race two oneshots into the same control slot.
    {
        let ctl = state.proxy_control.lock().await;
        if ctl.stop.is_some() {
            warn!("capture supervisor: backend already running, ignoring spawn");
            return Ok(());
        }
    }
    let (mode, proxy_cfg, chromium_cfg, platform) = {
        let cfg = state.config.read().await;
        (
            cfg.capture.mode,
            cfg.proxy.clone(),
            cfg.capture.chromium.clone(),
            cfg.platform.kind,
        )
    };

    // Build a fresh shutdown token for this run. Stored in
    // `proxy_control.stop` as a oneshot-via-Notify shim: command-side
    // `stop_proxy` calls `notify_waiters()` on `force_close` for the
    // hudsucker WS-kick AND fires the oneshot; we plumb both here.
    let (shutdown_token, shutdown_notify) = ShutdownToken::new();

    // Build backend per mode.
    let backend: Box<dyn CaptureBackend> = match mode {
        CaptureMode::Mitm => {
            let force_close = {
                let ctl = state.proxy_control.lock().await;
                ctl.force_close.clone()
            };
            Box::new(HudsuckerBackend::new(proxy_cfg.clone(), force_close))
        }
        CaptureMode::Chromium => Box::new(ChromiumBackend::new(chromium_cfg)),
    };
    let descriptor = backend.descriptor();

    // Status: emit Running with a descriptive label up front so the UI
    // sees an immediate transition from Stopped → Running. Backends that
    // fail mid-startup (e.g. Chromium can't find Chrome) will flip back
    // to Error from the spawned task.
    let label = match descriptor.kind {
        CaptureKind::Mitm => proxy_cfg.addr.clone(),
        CaptureKind::Chromium => format!("chromium ({})", descriptor.label),
    };
    let running_status = ProxyStatus::Running { addr: label.clone() };
    {
        let mut ctl = state.proxy_control.lock().await;
        // `stop` is filled via shutdown_notify below once we've spawned.
        // Reset the existing oneshot — caller already verified backend wasn't running.
        ctl.status = running_status.clone();
    }
    let _ = state.proxy_status_bus.send(running_status);

    let ctx = CaptureCtx {
        session: state.log_session.clone(),
        platform,
        mjai_bus: state.mjai_bus.clone(),
        notify_bus: state.notify_bus.clone(),
    };
    let proxy_bus = state.proxy_status_bus.clone();
    let control = state.proxy_control.clone();

    // Splice shutdown_notify into a oneshot-shaped trigger for
    // `proxy_control.stop`. The existing API (commands::stop_proxy)
    // sends `()` on the oneshot; we flip notify_notify_one in response.
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut ctl = state.proxy_control.lock().await;
        ctl.stop = Some(stop_tx);
    }
    let shutdown_notify_for_relay = shutdown_notify.clone();
    tokio::spawn(async move {
        let _ = (&mut stop_rx).await;
        shutdown_notify_for_relay.notify_waiters();
    });

    info!(
        "capture supervisor: starting {} backend ({})",
        descriptor.kind.as_str(),
        label
    );
    tokio::spawn(async move {
        let result = backend.run(ctx, shutdown_token).await;
        let next = match &result {
            Ok(()) => {
                info!("capture supervisor: stopped cleanly");
                ProxyStatus::Stopped
            }
            Err(e) => {
                error!("capture supervisor: {e:#}");
                ProxyStatus::Error {
                    addr: Some(label.clone()),
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
