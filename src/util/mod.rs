use std::path::{Path, PathBuf};

/// Resolve a configured directory path with the standard fallback chain:
///
/// 1. Absolute path → used as-is
/// 2. `<exe_dir>/<configured>` if it exists
/// 3. `<cwd>/<configured>` if it exists
/// 4. Otherwise return `<exe_dir>/<configured>` (preferred), falling back to
///    `<cwd>/<configured>` if the executable path is unavailable. The caller
///    is responsible for creating the directory.
pub fn resolve_dir(configured: &Path) -> PathBuf {
    if configured.is_absolute() {
        return configured.to_path_buf();
    }

    let exe_candidate = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(configured)));

    if let Some(p) = &exe_candidate {
        if p.exists() {
            return p.clone();
        }
    }

    let cwd_candidate = configured.to_path_buf();
    if cwd_candidate.exists() {
        return cwd_candidate;
    }

    exe_candidate.unwrap_or(cwd_candidate)
}
