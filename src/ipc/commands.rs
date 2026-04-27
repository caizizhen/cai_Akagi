//! `#[tauri::command]` handlers exposed to the frontend.
//!
//! Errors are returned as `String` because Tauri serializes command
//! errors via `Display` and most call sites just want a human message in
//! a toast. Keep the JSON shape conservative — clients lock onto field
//! names quickly and renames break dashboards.

use crate::analysis::result::AnalysisResult;
use crate::bot::BotRegistry;
use crate::config::AppConfig;
use crate::game_state::mahgen_view::MahgenView;
use crate::game_state::snapshot::GameStateSnapshot;
use crate::ipc::proxy_supervisor::spawn_proxy_supervisor;
use crate::ipc::state::AppState;
use crate::schema::{BotInfo, Notification, Snapshot};
use crate::util::resolve_dir;
use std::path::{Path, PathBuf};
use tauri::State;

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> CmdResult<AppConfig> {
    Ok(state.config.read().await.clone())
}

/// Replace the entire config and persist it to the same file the app
/// loaded from. Subsystems that are already running (proxy, bot manager)
/// keep their old settings until restarted — frontend should warn.
#[tauri::command]
pub async fn update_config(
    new_config: AppConfig,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    persist_config(&new_config, &state.config_path).map_err(|e| e.to_string())?;
    *state.config.write().await = new_config;
    let _ = state.notify_bus.send(
        Notification::success("Config saved")
            .body("Restart affected subsystems for changes to take effect."),
    );
    Ok(())
}

#[tauri::command]
pub async fn list_bots(state: State<'_, AppState>) -> CmdResult<Vec<BotInfo>> {
    let dir = state.config.read().await.bot.dir.clone();
    let resolved = resolve_dir(Path::new(&dir));
    let registry = BotRegistry::scan(&resolved).map_err(|e| format!("scan bots: {e:#}"))?;
    Ok(registry
        .entries()
        .iter()
        .map(|e| BotInfo {
            name: e.name.clone(),
            dir: e.dir.to_string_lossy().into_owned(),
            has_pyproject: e.pyproject.is_some(),
        })
        .collect())
}

/// Update `bot.active` in config + persist. Doesn't restart the running
/// `BotManager` — that respawns on the next `start_game` event anyway.
#[tauri::command]
pub async fn set_active_bot(name: String, state: State<'_, AppState>) -> CmdResult<()> {
    {
        let mut cfg = state.config.write().await;
        cfg.bot.active = name.clone();
        persist_config(&cfg, &state.config_path).map_err(|e| e.to_string())?;
    }
    let _ = state
        .notify_bus
        .send(Notification::success(format!("Active bot set to {name}")));
    Ok(())
}

#[tauri::command]
pub async fn start_proxy(state: State<'_, AppState>) -> CmdResult<()> {
    let already_running = {
        let ctl = state.proxy_control.lock().await;
        ctl.stop.is_some()
    };
    if already_running {
        return Err("proxy already running".into());
    }
    spawn_proxy_supervisor((*state).clone())
        .await
        .map_err(|e| format!("start proxy: {e:#}"))
}

#[tauri::command]
pub async fn stop_proxy(state: State<'_, AppState>) -> CmdResult<()> {
    let stop = {
        let mut ctl = state.proxy_control.lock().await;
        ctl.stop.take()
    };
    match stop {
        Some(tx) => {
            // Receiver dropped means the task already exited — that's fine,
            // we still cleared `stop` and the status forwarder will catch
            // up via the next ProxyStatus emission.
            let _ = tx.send(());
            Ok(())
        }
        None => Err("proxy is not running".into()),
    }
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    let config = state.config.read().await.clone();
    let bot_status = state.bot_status.read().await.clone();
    let proxy_status = state.proxy_control.lock().await.status.clone();
    let log_dir = state.log_session.dir().to_path_buf();
    Ok(Snapshot {
        config,
        bot_status,
        proxy_status,
        log_dir,
    })
}

#[tauri::command]
pub async fn get_log_dir(state: State<'_, AppState>) -> CmdResult<PathBuf> {
    Ok(state.log_session.dir().to_path_buf())
}

/// Latest analysis output. `None` until the analysis runner has produced
/// at least one result for the current game.
#[tauri::command]
pub async fn get_analysis(state: State<'_, AppState>) -> CmdResult<Option<AnalysisResult>> {
    Ok(state.analysis_cache.read().await.clone())
}

/// Live game-state snapshot from the tracker. `None` before any
/// `start_game` event has been observed.
#[tauri::command]
pub async fn get_game_snapshot(
    state: State<'_, AppState>,
) -> CmdResult<Option<GameStateSnapshot>> {
    Ok(state.game_tracker.lock().await.snapshot())
}

/// Pre-encoded mahgen DSL strings ready for the frontend `<mah-gen>`
/// element. Built from the same snapshot as `get_game_snapshot` — call
/// whichever the UI surface prefers; both are O(34 tiles) to generate.
#[tauri::command]
pub async fn get_mahgen_view(state: State<'_, AppState>) -> CmdResult<Option<MahgenView>> {
    Ok(state
        .game_tracker
        .lock()
        .await
        .snapshot()
        .map(|s| MahgenView::from_snapshot(&s)))
}

fn persist_config(config: &AppConfig, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let body = toml::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, body)
}

/// Pre-builds the handler list for `tauri::generate_handler!`. Keep in
/// sync when adding commands.
#[macro_export]
macro_rules! ipc_handlers {
    () => {
        ::tauri::generate_handler![
            $crate::ipc::commands::get_config,
            $crate::ipc::commands::update_config,
            $crate::ipc::commands::list_bots,
            $crate::ipc::commands::set_active_bot,
            $crate::ipc::commands::start_proxy,
            $crate::ipc::commands::stop_proxy,
            $crate::ipc::commands::get_status,
            $crate::ipc::commands::get_log_dir,
            $crate::ipc::commands::get_analysis,
            $crate::ipc::commands::get_game_snapshot,
            $crate::ipc::commands::get_mahgen_view,
        ]
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn persist_config_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("config.toml");
        let mut cfg = AppConfig::default();
        cfg.bot.active = "mortal".into();
        cfg.proxy.addr = "127.0.0.1:9999".into();

        persist_config(&cfg, &path).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        let back: AppConfig = toml::from_str(&body).unwrap();
        assert_eq!(back.bot.active, "mortal");
        assert_eq!(back.proxy.addr, "127.0.0.1:9999");
    }
}
