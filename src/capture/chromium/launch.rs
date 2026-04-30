//! Spawn the controlled Chromium process and read its CDP endpoint.
//!
//! Lifecycle:
//! 1. `spawn` runs the chrome binary with `--user-data-dir`, `--remote-debugging-port=0`,
//!    and assorted "don't be annoying" flags.
//! 2. `wait_for_devtools_port` polls `<user-data-dir>/DevToolsActivePort`
//!    (Chrome writes it ~50ms after start) and returns a `ws://...` URL
//!    suitable for `chromiumoxide::Browser::connect`.
//! 3. `terminate` does the staged shutdown: SIGTERM (Unix) / `taskkill`
//!    (Windows) → wait → SIGKILL.

use crate::config::ChromiumConfig;
use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::time::Duration;
use tokio::process::{Child, Command};
use tracing::{debug, warn};

const DEVTOOLS_FILE: &str = "DevToolsActivePort";
const PORT_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const PORT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const TERM_GRACE: Duration = Duration::from_secs(5);
const KILL_GRACE: Duration = Duration::from_secs(2);

pub fn spawn(exe: &Path, profile: &Path, cfg: &ChromiumConfig) -> Result<Child> {
    let mut cmd = Command::new(exe);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-features=TranslateUI,InterestFeedContentSuggestions")
        .arg("--disable-search-engine-choice-screen")
        .arg("--disable-component-update")
        .arg("--disable-background-networking")
        .arg("--disable-sync")
        .arg("--metrics-recording-only")
        .arg("--no-pings")
        .arg("--mute-audio");
    if cfg!(target_os = "linux") {
        // Chromium occasionally crashes on Linux when /dev/shm is small (e.g. Docker).
        cmd.arg("--disable-dev-shm-usage");
    }
    for extra in &cfg.extra_args {
        cmd.arg(extra);
    }
    if !cfg.start_url.is_empty() {
        cmd.arg(&cfg.start_url);
    }
    // Stale port file from a previous run would confuse the polling read.
    let port_file = profile.join(DEVTOOLS_FILE);
    if port_file.exists() {
        let _ = std::fs::remove_file(&port_file);
    }
    cmd.kill_on_drop(true);
    let child = cmd.spawn().with_context(|| {
        format!("failed to spawn chromium binary at {}", exe.display())
    })?;
    debug!("spawned chromium pid={:?}", child.id());
    Ok(child)
}

/// Poll the `DevToolsActivePort` file until Chromium writes its CDP
/// endpoint, then return the `ws://...` URL.
pub async fn wait_for_devtools_port(profile: &Path) -> Result<String> {
    let port_file = profile.join(DEVTOOLS_FILE);
    let start = std::time::Instant::now();
    loop {
        if let Ok(body) = std::fs::read_to_string(&port_file) {
            if let Some(endpoint) = parse_devtools_port(&body) {
                return Ok(endpoint);
            }
        }
        if start.elapsed() > PORT_WAIT_TIMEOUT {
            return Err(anyhow!(
                "timed out after {:?} waiting for {}",
                PORT_WAIT_TIMEOUT,
                port_file.display()
            ));
        }
        tokio::time::sleep(PORT_POLL_INTERVAL).await;
    }
}

/// Parse the two-line `DevToolsActivePort` content into a `ws://...` URL.
/// Returns `None` if the file is partially written (Chrome writes the
/// port first, then the path on a separate line).
pub fn parse_devtools_port(body: &str) -> Option<String> {
    let mut lines = body.lines();
    let port_line = lines.next()?.trim();
    let path_line = lines.next()?.trim();
    if port_line.is_empty() || path_line.is_empty() {
        return None;
    }
    let port: u16 = port_line.parse().ok()?;
    Some(format!("ws://127.0.0.1:{port}{path_line}"))
}

/// Best-effort staged shutdown: try a polite term, escalate to kill if
/// the child doesn't exit. Always returns; never panics.
pub async fn terminate(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let _ = std::process::Command::new("/bin/kill")
                .args(["-TERM", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
    #[cfg(windows)]
    {
        if let Some(pid) = child.id() {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status();
        }
    }
    if tokio::time::timeout(TERM_GRACE, child.wait()).await.is_err() {
        warn!("chromium did not exit after SIGTERM, sending SIGKILL");
        let _ = child.kill().await;
        let _ = tokio::time::timeout(KILL_GRACE, child.wait()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devtools_port_well_formed() {
        let body = "9222\n/devtools/browser/abc-123\n";
        assert_eq!(
            parse_devtools_port(body),
            Some("ws://127.0.0.1:9222/devtools/browser/abc-123".to_string())
        );
    }

    #[test]
    fn returns_none_for_partial_write() {
        assert!(parse_devtools_port("").is_none());
        assert!(parse_devtools_port("9222").is_none()); // path line missing
        assert!(parse_devtools_port("\n").is_none());
    }

    #[test]
    fn rejects_garbage_port() {
        assert!(parse_devtools_port("not-a-port\n/path\n").is_none());
    }
}
