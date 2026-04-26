//! Cross-platform startup hooks. Called once from `run()` before any GUI
//! subsystem (Tauri / webkit2gtk / wry / WebView2) is initialised.

/// Apply environment workarounds required for the current OS / display server.
///
/// - **Linux + Wayland**: webkit2gtk's default dmabuf renderer regularly trips
///   `Gdk-Message: Error 71 (Protocol error)` against modern Wayland compositors
///   (Mutter / KWin / Hyprland). Force-disable it. Also disable the GL
///   compositing mode as a secondary fallback. Both are no-ops on X11.
/// - **Windows / macOS**: nothing required at the moment.
pub fn setup() {
    #[cfg(target_os = "linux")]
    setup_linux();
}

#[cfg(target_os = "linux")]
fn setup_linux() {
    for var in [
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
    ] {
        if std::env::var_os(var).is_none() {
            std::env::set_var(var, "1");
        }
    }
}
