//! Chromium capture backend.
//!
//! Launches a Chromium-family browser with `--user-data-dir` (so it
//! doesn't collide with the user's existing Chrome) and `--remote-debugging-port=0`,
//! then connects to it via the Chrome DevTools Protocol and intercepts
//! `Network.webSocketFrameReceived/Sent` for binary frames. Frames are
//! routed into the platform [`crate::bridge::Bridge`] just as the
//! hudsucker backend does.
//!
//! No CA cert. No system proxy. The user just plays the game in the
//! Akagi-spawned browser window.

pub mod cdp;
pub mod cft;
pub mod detect;
pub mod launch;
pub mod profile;

use super::{CaptureBackend, CaptureCtx, CaptureDescriptor, CaptureKind, ShutdownToken};
use crate::capture::flow::FlowBridges;
use crate::config::ChromiumConfig;
use crate::event_bus::NotifyBus;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

pub struct ChromiumBackend {
    cfg: ChromiumConfig,
}

impl ChromiumBackend {
    pub fn new(cfg: ChromiumConfig) -> Self {
        Self { cfg }
    }
}

/// Pick the browser binary to launch for Chromium capture.
///
/// 1. Explicit `cfg.executable` if set (must exist).
/// 2. Installed Chrome-for-Testing when present (stable CDP with chromiumoxide).
/// 3. Otherwise download CfT once ([`cft::install`]); on failure, if
///    `force_cft` is false fall back to the first system browser; if true,
///    return an error.
async fn resolve_launch_executable(cfg: &ChromiumConfig, notify: &NotifyBus) -> Result<PathBuf> {
    if !cfg.executable.is_empty() {
        let p = PathBuf::from(&cfg.executable);
        if !p.exists() {
            anyhow::bail!(
                "configured chromium executable does not exist: {}",
                p.display()
            );
        }
        return Ok(p);
    }

    let pinned = cft::Channel::parse(&cfg.cft_channel);

    if let Some(cft_exe) = cft::installed_executable(&pinned) {
        info!(
            "chromium capture: using Chrome-for-Testing at {}",
            cft_exe.display()
        );
        return Ok(cft_exe);
    }

    info!(
        "Chrome-for-Testing not installed; downloading stable channel for capture (~150 MB). \
         To use another browser instead, set capture.chromium.executable in config."
    );

    match cft::install(&pinned, notify).await {
        Ok(_) => {}
        Err(e) if !cfg.force_cft => {
            warn!(
                "Chrome-for-Testing download/install failed: {e:#}; falling back to system browser \
                 (recent Google Chrome may break CDP — install CfT from Settings → Capture when online)"
            );
        }
        Err(e) => {
            return Err(e).context(
                "Chrome-for-Testing is required (force_cft) but automatic install failed; \
                 fix network or install from Settings → Capture",
            );
        }
    }

    if let Some(cft_exe) = cft::installed_executable(&pinned) {
        info!(
            "chromium capture: using Chrome-for-Testing at {}",
            cft_exe.display()
        );
        return Ok(cft_exe);
    }

    if cfg.force_cft {
        anyhow::bail!(
            "Chrome-for-Testing is required (force_cft) but no usable install was found after download. \
             Open Settings → Capture to install manually, or set capture.chromium.force_cft to false."
        );
    }

    if let Some(b) = detect::detect_system_browsers().into_iter().next() {
        warn!(
            "chromium capture: using system browser at {}",
            b.path.display()
        );
        return Ok(b.path);
    }

    anyhow::bail!(
        "no Chromium-family browser detected and Chrome-for-Testing could not be installed. \
         Open Settings → Capture to install Chrome for Testing, or set capture.chromium.executable."
    )
}

#[async_trait]
impl CaptureBackend for ChromiumBackend {
    async fn run(self: Box<Self>, ctx: CaptureCtx, shutdown: ShutdownToken) -> Result<()> {
        let exe = resolve_launch_executable(&self.cfg, &ctx.notify_bus)
            .await
            .context("resolving chromium executable")?;
        let profile_dir = profile::resolve_profile_dir(&self.cfg.user_data_dir)?;
        std::fs::create_dir_all(&profile_dir)
            .with_context(|| format!("creating chromium profile dir {}", profile_dir.display()))?;
        if let Err(e) = profile::clear_stale_singleton(&profile_dir) {
            warn!(
                "singleton-lock cleanup at {} failed: {e:#}",
                profile_dir.display()
            );
        }

        info!(
            "chromium backend starting: exe={} profile={}",
            exe.display(),
            profile_dir.display()
        );

        let mut child =
            launch::spawn(&exe, &profile_dir, &self.cfg).context("launching chromium")?;

        let cdp_endpoint = launch::wait_for_devtools_port(&profile_dir)
            .await
            .context("reading DevToolsActivePort (chromium failed to start?)")?;
        info!("chromium CDP endpoint: {cdp_endpoint}");

        let bridges = Arc::new(FlowBridges::<cdp::FlowKey>::new(
            ctx.session.clone(),
            ctx.platform,
        ));

        let cdp_run = cdp::run(
            &cdp_endpoint,
            bridges.clone(),
            ctx.mjai_bus.clone(),
            ctx.session.inspector(),
            ctx.autoplay.clone(),
        );
        let mut cdp_fut = Box::pin(cdp_run);
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);

        let result = tokio::select! {
            biased;
            _ = &mut shutdown_fut => {
                info!("chromium backend: shutdown requested");
                Ok(())
            }
            r = &mut cdp_fut => {
                match &r {
                    Ok(()) => info!("chromium backend: CDP loop exited cleanly"),
                    Err(e) => warn!("chromium backend: CDP loop error: {e:#}"),
                }
                r
            }
            status = child.wait() => {
                match status {
                    Ok(s) => {
                        warn!("chromium backend: browser exited (status {s})");
                        Err(anyhow::anyhow!("browser exited unexpectedly: {s}"))
                    }
                    Err(e) => Err(anyhow::anyhow!("child wait error: {e}")),
                }
            }
        };

        // Best-effort browser shutdown.
        launch::terminate(&mut child).await;
        result
    }

    fn descriptor(&self) -> CaptureDescriptor {
        let label = if self.cfg.executable.is_empty() {
            "auto-detect".to_string()
        } else {
            self.cfg.executable.clone()
        };
        CaptureDescriptor {
            kind: CaptureKind::Chromium,
            label,
        }
    }
}
