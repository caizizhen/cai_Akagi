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
pub mod detect;
pub mod launch;
pub mod profile;

use super::{CaptureBackend, CaptureCtx, CaptureDescriptor, CaptureKind, ShutdownToken};
use crate::capture::flow::FlowBridges;
use crate::config::ChromiumConfig;
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

    /// Resolve the chrome executable: explicit `cfg.executable`, or first
    /// detected system browser, or installed Chrome-for-Testing. Returns
    /// `Err` with a user-friendly message if nothing usable was found.
    fn resolve_executable(&self) -> Result<PathBuf> {
        if !self.cfg.executable.is_empty() {
            let p = PathBuf::from(&self.cfg.executable);
            if !p.exists() {
                anyhow::bail!(
                    "configured chromium executable does not exist: {}",
                    p.display()
                );
            }
            return Ok(p);
        }

        if !self.cfg.force_cft {
            if let Some(b) = detect::detect_system_browsers().into_iter().next() {
                return Ok(b.path);
            }
        }

        // CfT fallback comes in Phase 2; for v1 we surface a clean error
        // rather than silently failing.
        anyhow::bail!(
            "no Chromium-family browser detected and Chrome-for-Testing fallback is not yet implemented. \
             Install Google Chrome / Chromium / Edge, or set capture.chromium.executable explicitly."
        )
    }
}

#[async_trait]
impl CaptureBackend for ChromiumBackend {
    async fn run(self: Box<Self>, ctx: CaptureCtx, shutdown: ShutdownToken) -> Result<()> {
        let exe = self.resolve_executable().context("resolving chromium executable")?;
        let profile_dir = profile::resolve_profile_dir(&self.cfg.user_data_dir)?;
        std::fs::create_dir_all(&profile_dir)
            .with_context(|| format!("creating chromium profile dir {}", profile_dir.display()))?;
        if let Err(e) = profile::clear_stale_singleton(&profile_dir) {
            warn!("singleton-lock cleanup at {} failed: {e:#}", profile_dir.display());
        }

        info!(
            "chromium backend starting: exe={} profile={}",
            exe.display(),
            profile_dir.display()
        );

        let mut child = launch::spawn(&exe, &profile_dir, &self.cfg)
            .context("launching chromium")?;

        let cdp_endpoint = launch::wait_for_devtools_port(&profile_dir).await
            .context("reading DevToolsActivePort (chromium failed to start?)")?;
        info!("chromium CDP endpoint: {cdp_endpoint}");

        let bridges = Arc::new(FlowBridges::<cdp::FlowKey>::new(
            ctx.session.clone(),
            ctx.platform,
        ));

        let cdp_run = cdp::run(&cdp_endpoint, bridges.clone(), ctx.mjai_bus.clone());
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
