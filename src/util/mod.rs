use std::path::{Path, PathBuf};

/// True when running inside an AppImage runtime (read-only squashfs mount).
/// AppImage sets `$APPIMAGE` to the .AppImage path before exec.
pub fn is_appimage() -> bool {
    std::env::var_os("APPIMAGE").is_some()
}

/// `~/.config/akagi` (Linux) or platform equivalent. None if unresolvable.
/// All AppImage runtime paths (config, logs, bot dir, CA dir) anchor here.
pub fn user_config_root() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("akagi"))
}

/// Strip a leading `./` so `./logs` joins cleanly under a user dir.
pub fn strip_leading_dot(p: &Path) -> &Path {
    p.strip_prefix("./").unwrap_or(p)
}

/// `<user_config_root>/<name>` if available. Use this for runtime data
/// that must always live in a writable XDG-style location regardless of
/// AppImage / system-install / cwd. Unlike [`resolve_dir`], there is no
/// exe-dir-first fallback — callers like the Chromium profile and CfT
/// install dir explicitly want the user dir every time.
pub fn user_subdir(name: &str) -> Option<PathBuf> {
    user_config_root().map(|r| r.join(name))
}

/// Resolve a configured directory path with the standard fallback chain:
///
/// 1. Absolute path → used as-is
/// 2. `<exe_dir>/<configured>` if it exists
/// 3. `<cwd>/<configured>` if it exists
/// 4. Under AppImage: `<user_data_root>/<configured>` (writable XDG location).
/// 5. Otherwise return `<exe_dir>/<configured>` (preferred), falling back to
///    `<cwd>/<configured>` if the executable path is unavailable. The caller
///    is responsible for creating the directory.
pub fn resolve_dir(configured: &Path) -> PathBuf {
    resolve_dir_inner(configured, is_appimage(), user_config_root())
}

fn resolve_dir_inner(configured: &Path, appimage: bool, user_root: Option<PathBuf>) -> PathBuf {
    if configured.is_absolute() {
        return configured.to_path_buf();
    }

    let exe_candidate = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(strip_leading_dot(configured))));

    if let Some(p) = &exe_candidate {
        if p.exists() {
            return p.clone();
        }
    }

    let cwd_candidate = configured.to_path_buf();
    if cwd_candidate.exists() {
        return cwd_candidate;
    }

    if appimage {
        if let Some(root) = user_root {
            return root.join(strip_leading_dot(configured));
        }
    }

    exe_candidate.unwrap_or(cwd_candidate)
}

/// True if `dir` looks like an Akagi bot root: at least one **registry-eligible**
/// subdirectory containing `bot.py` (same rules as [`crate::bot::registry::BotRegistry`]).
fn mjai_bot_root_has_bots(dir: &Path) -> bool {
    std::fs::read_dir(dir).map_or(false, |mut it| {
        it.any(|e| {
            e.ok()
                .map(|ent| {
                    let path = ent.path();
                    if !path.is_dir() {
                        return false;
                    }
                    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                        return false;
                    };
                    if name.starts_with('.') || name.starts_with("__") || name == "base" {
                        return false;
                    }
                    path.join("bot.py").is_file()
                })
                .unwrap_or(false)
        })
    })
}

fn is_default_mjai_bot_relative(configured: &Path) -> bool {
    strip_leading_dot(configured).as_os_str() == std::path::Path::new("mjai_bot").as_os_str()
}

/// When running from a Cargo build, the exe is under `target/<profile>/` or
/// `target/<profile>/deps/` while the repo keeps `mjai_bot/` at the workspace
/// root — [`resolve_dir`] may pick an empty `target/debug/mjai_bot` if that folder exists.
pub(crate) fn workspace_mjai_bot_from_exe_path(exe: &Path) -> Option<PathBuf> {
    let mut cur = exe.parent()?;
    loop {
        if cur.file_name()?.to_str()? == "target" {
            let workspace = cur.parent()?;
            let candidate = workspace.join("mjai_bot");
            return candidate.is_dir().then_some(candidate);
        }
        cur = cur.parent()?;
    }
}

fn workspace_mjai_bot_from_exe() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(workspace_mjai_bot_from_exe_path)
}

/// Like [`resolve_dir`] for `bot.dir`, but when the default relative
/// `mjai_bot` resolves to a directory with **no** `*/bot.py` children, fall
/// back to `<workspace>/mjai_bot` for the common `cargo run` layout.
pub fn resolve_mjai_bot_dir(configured: &Path) -> PathBuf {
    let primary = resolve_dir(configured);
    if mjai_bot_root_has_bots(&primary) {
        return primary;
    }
    if is_default_mjai_bot_relative(configured) {
        if let Some(ws) = workspace_mjai_bot_from_exe() {
            if mjai_bot_root_has_bots(&ws) {
                return ws;
            }
        }
    }
    primary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_leading_dot_removes_dotslash() {
        assert_eq!(strip_leading_dot(Path::new("./logs")), Path::new("logs"));
        assert_eq!(strip_leading_dot(Path::new("logs")), Path::new("logs"));
        assert_eq!(strip_leading_dot(Path::new("./a/b")), Path::new("a/b"));
    }

    #[test]
    fn appimage_routes_relative_path_to_user_config() {
        let user_root = PathBuf::from("/home/u/.config/akagi");
        let resolved = resolve_dir_inner(Path::new("./logs"), true, Some(user_root.clone()));
        assert_eq!(resolved, user_root.join("logs"));
    }

    #[test]
    fn appimage_preserves_absolute_path() {
        let resolved = resolve_dir_inner(
            Path::new("/var/log/akagi"),
            true,
            Some(PathBuf::from("/home/u/.config/akagi")),
        );
        assert_eq!(resolved, PathBuf::from("/var/log/akagi"));
    }

    #[test]
    fn non_appimage_does_not_use_user_config_for_dirs() {
        // Non-appimage with a path that doesn't exist anywhere should NOT
        // fall back to user_root; it returns the exe-relative candidate.
        let user_root = PathBuf::from("/home/u/.config/akagi");
        let resolved = resolve_dir_inner(
            Path::new("./nonexistent-xyz"),
            false,
            Some(user_root.clone()),
        );
        assert!(!resolved.starts_with(&user_root));
    }

    #[test]
    fn workspace_mjai_bot_from_exe_path_finds_repo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path();
        let mjai = workspace.join("mjai_bot");
        std::fs::create_dir_all(&mjai).unwrap();
        let exe = workspace.join("target").join("debug").join("akagi.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        assert_eq!(super::workspace_mjai_bot_from_exe_path(&exe).unwrap(), mjai);
    }

    #[test]
    fn workspace_mjai_bot_from_exe_path_finds_repo_root_from_deps_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path();
        let mjai = workspace.join("mjai_bot");
        std::fs::create_dir_all(&mjai).unwrap();
        let exe = workspace
            .join("target")
            .join("debug")
            .join("deps")
            .join("akagi-deadbeef.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        assert_eq!(super::workspace_mjai_bot_from_exe_path(&exe).unwrap(), mjai);
    }

    #[test]
    fn relative_dot_path_does_not_leak_into_absolute() {
        // Regression: `./logs` joined with absolute exe-parent used to keep
        // the literal `./` component, producing paths like
        // `/home/.../akagi/./logs/...` that surfaced in the UI as broken.
        let resolved = resolve_dir_inner(Path::new("./logs"), false, None);
        let s = resolved.to_string_lossy();
        assert!(!s.contains("/./"), "got: {s}");
        assert!(!s.contains("\\.\\"), "got: {s}");
    }
}
