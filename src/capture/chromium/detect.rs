//! Locate a Chromium-family browser on the user's system.
//!
//! Returns candidates in priority order: Chrome → Edge → Brave → Chromium
//! → installed Chrome-for-Testing. Probing is filesystem-stat only — no
//! launching, no version checks — so it's cheap to call from the Settings
//! page on every render.
//!
//! Per-OS strategy:
//! - **Linux**: `which::which` for the well-known names, plus a few
//!   distro-specific fixed paths. Flatpak Chrome is *intentionally
//!   skipped* — its sandbox refuses external `--user-data-dir`.
//! - **macOS**: well-known `.app` bundle paths under `/Applications` and
//!   `~/Applications`.
//! - **Windows**: `reg query` for the App Paths key, with fixed-path
//!   fallback under `%PROGRAMFILES%` / `%PROGRAMFILES(X86)%` /
//!   `%LOCALAPPDATA%`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserKind {
    Chrome,
    Edge,
    Brave,
    Chromium,
    /// Locally installed Chrome-for-Testing. CfT detection is best-effort
    /// in v1 (Phase 2 owns the install side; this just notices what's there).
    ChromeForTesting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedBrowser {
    pub kind: BrowserKind,
    pub path: PathBuf,
}

/// Probe the system for installed Chromium-family browsers, in priority
/// order. Empty result means "user has no compatible browser".
pub fn detect_system_browsers() -> Vec<DetectedBrowser> {
    let mut found = Vec::new();
    detect_into(&mut found);
    found.dedup_by(|a, b| a.path == b.path);
    found
}

#[cfg(target_os = "linux")]
fn detect_into(found: &mut Vec<DetectedBrowser>) {
    use BrowserKind::*;
    // (which-name, kind) — try PATH first because distros put the binary
    // in different places (snap, flatpak excluded).
    let path_probes = [
        ("google-chrome", Chrome),
        ("google-chrome-stable", Chrome),
        ("microsoft-edge", Edge),
        ("brave-browser", Brave),
        ("chromium", Chromium),
        ("chromium-browser", Chromium),
    ];
    for (name, kind) in path_probes {
        if let Ok(p) = which::which(name) {
            // Skip flatpak shim — sandbox refuses external --user-data-dir.
            if p.to_string_lossy().contains("/flatpak/") {
                continue;
            }
            found.push(DetectedBrowser { kind, path: p });
        }
    }
    // Fixed paths as backup (some installs don't expose to PATH).
    let fixed = [
        ("/usr/bin/google-chrome", Chrome),
        ("/usr/bin/google-chrome-stable", Chrome),
        ("/usr/bin/microsoft-edge", Edge),
        ("/usr/bin/brave-browser", Brave),
        ("/usr/bin/chromium", Chromium),
        ("/usr/bin/chromium-browser", Chromium),
        ("/snap/bin/chromium", Chromium),
    ];
    for (p, kind) in fixed {
        let pb = PathBuf::from(p);
        if pb.exists() {
            found.push(DetectedBrowser { kind, path: pb });
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_into(found: &mut Vec<DetectedBrowser>) {
    use BrowserKind::*;
    let bundles = [
        ("Google Chrome.app", "Google Chrome", Chrome),
        ("Microsoft Edge.app", "Microsoft Edge", Edge),
        ("Brave Browser.app", "Brave Browser", Brave),
        ("Chromium.app", "Chromium", Chromium),
        (
            "Google Chrome for Testing.app",
            "Google Chrome for Testing",
            ChromeForTesting,
        ),
    ];
    let prefixes = [
        PathBuf::from("/Applications"),
        dirs::home_dir()
            .map(|h| h.join("Applications"))
            .unwrap_or_default(),
    ];
    for prefix in &prefixes {
        for (bundle, exe, kind) in &bundles {
            let p = prefix.join(bundle).join("Contents/MacOS").join(exe);
            if p.exists() {
                found.push(DetectedBrowser {
                    kind: kind.clone(),
                    path: p,
                });
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn detect_into(found: &mut Vec<DetectedBrowser>) {
    use BrowserKind::*;
    // Try registry App Paths first.
    if let Some(p) = reg_query_app_paths("chrome.exe") {
        found.push(DetectedBrowser { kind: Chrome, path: p });
    }
    if let Some(p) = reg_query_app_paths("msedge.exe") {
        found.push(DetectedBrowser { kind: Edge, path: p });
    }
    if let Some(p) = reg_query_app_paths("brave.exe") {
        found.push(DetectedBrowser { kind: Brave, path: p });
    }

    // Fixed paths.
    let env = |k: &str| std::env::var(k).ok().map(PathBuf::from);
    let pf = env("PROGRAMFILES");
    let pf86 = env("PROGRAMFILES(X86)");
    let local = env("LOCALAPPDATA");
    for (relative, kind) in [
        ("Google\\Chrome\\Application\\chrome.exe", Chrome),
        ("Microsoft\\Edge\\Application\\msedge.exe", Edge),
        ("BraveSoftware\\Brave-Browser\\Application\\brave.exe", Brave),
        ("Chromium\\Application\\chrome.exe", Chromium),
    ] {
        for base in [&pf, &pf86, &local].iter().filter_map(|x| x.as_ref()) {
            let p = base.join(relative);
            if p.exists() {
                found.push(DetectedBrowser {
                    kind: kind.clone(),
                    path: p,
                });
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn reg_query_app_paths(exe: &str) -> Option<PathBuf> {
    let key = format!(
        "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
        exe
    );
    let out = std::process::Command::new("reg")
        .args(["query", &key, "/ve"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    // Output looks like "  (Default)    REG_SZ    C:\\path\\to\\chrome.exe"
    for line in s.lines() {
        let line = line.trim();
        if let Some(idx) = line.find("REG_SZ") {
            let value = line[idx + "REG_SZ".len()..].trim();
            let p = PathBuf::from(value.trim_matches('"'));
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_into(_found: &mut Vec<DetectedBrowser>) {
    // No detection on unsupported platforms.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_dedup() {
        // Whatever this dev box has, dedup must hold.
        let v = detect_system_browsers();
        let mut paths: Vec<_> = v.iter().map(|b| b.path.clone()).collect();
        paths.sort();
        let unique = paths.clone();
        let mut u = unique;
        u.dedup();
        assert_eq!(paths.len(), u.len(), "detect_system_browsers must dedup");
    }
}
