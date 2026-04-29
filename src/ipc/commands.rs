//! `#[tauri::command]` handlers exposed to the frontend.
//!
//! Errors are returned as `String` because Tauri serializes command
//! errors via `Display` and most call sites just want a human message in
//! a toast. Keep the JSON shape conservative — clients lock onto field
//! names quickly and renames break dashboards.

use crate::analysis::result::AnalysisResult;
use crate::bot::install::{self, GithubInstallSpec};
use crate::bot::manifest::{self, BotSource};
use crate::bot::runtime;
use crate::bot::sync_guard::SyncGuard;
use crate::bot::{BotEntry, BotRegistry};
use crate::config::AppConfig;
use crate::game_state::mahgen_view::MahgenView;
use crate::game_state::snapshot::GameStateSnapshot;
use crate::ipc::proxy_supervisor::spawn_proxy_supervisor;
use crate::ipc::state::AppState;
use crate::schema::{BotInfo, BotSettings, Notification, Snapshot};
use crate::util::resolve_dir;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;

fn entry_to_info(e: &BotEntry) -> BotInfo {
    BotInfo {
        name: e.name.clone(),
        dir: e.dir.to_string_lossy().into_owned(),
        has_pyproject: e.pyproject.is_some(),
        manifest: e.manifest.clone(),
    }
}

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
    Ok(registry.entries().iter().map(entry_to_info).collect())
}

/// Read the merged settings (manifest + on-disk values) for one bot.
/// Returns an error when the bot does not exist or has no manifest —
/// frontend should hide the settings panel for manifest-less bots and
/// avoid calling this command for them.
#[tauri::command]
pub async fn get_bot_settings(
    name: String,
    state: State<'_, AppState>,
) -> CmdResult<BotSettings> {
    let dir = state.config.read().await.bot.dir.clone();
    let resolved = resolve_dir(Path::new(&dir));
    let registry = BotRegistry::scan(&resolved).map_err(|e| format!("scan bots: {e:#}"))?;
    let entry = registry
        .find(&name)
        .ok_or_else(|| format!("bot {name:?} not found"))?;
    let manifest = entry
        .manifest
        .clone()
        .ok_or_else(|| format!("bot {name:?} has no manifest.toml"))?;
    let values = manifest::load_values(&entry.dir, &manifest)
        .map_err(|e| format!("load settings: {e:#}"))?;
    Ok(BotSettings { manifest, values })
}

/// Persist user-edited settings for one bot. Validates the values against
/// the manifest before writing — wrong type, out-of-range numeric, and
/// unknown enum choice all surface as command errors.
///
/// New values take effect on the next bot spawn (i.e. the next
/// `start_game` event). The currently-running subprocess keeps its old
/// values; document this caveat in the UI.
#[tauri::command]
pub async fn update_bot_settings(
    name: String,
    values: BTreeMap<String, serde_json::Value>,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    let dir = state.config.read().await.bot.dir.clone();
    let resolved = resolve_dir(Path::new(&dir));
    let registry = BotRegistry::scan(&resolved).map_err(|e| format!("scan bots: {e:#}"))?;
    let entry = registry
        .find(&name)
        .ok_or_else(|| format!("bot {name:?} not found"))?;
    let manifest = entry
        .manifest
        .as_ref()
        .ok_or_else(|| format!("bot {name:?} has no manifest.toml"))?;
    manifest::save_values(&entry.dir, manifest, &values)
        .map_err(|e| format!("save settings: {e:#}"))?;
    let _ = state
        .notify_bus
        .send(Notification::success(format!("{name} settings saved")));
    Ok(())
}

/// Update the active bot for a given mode (`"4p"` or `"3p"`) in config +
/// persist. Doesn't restart the running `BotManager` — that respawns on the
/// next `start_game` event anyway, and picks the matching mode bot then.
///
/// Empty `name` clears that mode's active bot (analysis-only in that mode).
#[tauri::command]
pub async fn set_active_bot(
    mode: String,
    name: String,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    {
        let mut cfg = state.config.write().await;
        match mode.as_str() {
            "4p" => cfg.bot.active_4p = name.clone(),
            "3p" => cfg.bot.active_3p = name.clone(),
            other => return Err(format!("unknown mode {other:?}; expected \"4p\" or \"3p\"")),
        }
        persist_config(&cfg, &state.config_path).map_err(|e| e.to_string())?;
    }
    let label = if name.is_empty() {
        format!("{mode} bot cleared")
    } else {
        format!("Active {mode} bot set to {name}")
    };
    let _ = state.notify_bus.send(Notification::success(label));
    Ok(())
}

/// Install a bot by downloading the latest release zip from a GitHub
/// repository. Refuses to overwrite an existing `mjai_bot/<name>/` —
/// the user must remove it first via the file browser. The installer
/// reports progress through `NotifyBus` with sticky id
/// `bot-install-<name>`.
#[tauri::command]
pub async fn install_bot_from_github(
    repo: String,
    asset_glob: Option<String>,
    name: Option<String>,
    state: State<'_, AppState>,
) -> CmdResult<BotInfo> {
    let dir = state.config.read().await.bot.dir.clone();
    let resolved = resolve_dir(Path::new(&dir));
    std::fs::create_dir_all(&resolved)
        .map_err(|e| format!("create bot dir {}: {e}", resolved.display()))?;

    let spec = GithubInstallSpec {
        repo,
        asset_glob,
        name,
    };
    let entry = install::install_from_github_release(
        spec,
        &resolved,
        &state.notify_bus,
        state.runtime.as_ref(),
    )
    .await
    .map_err(|e| format!("install: {e:#}"))?;
    Ok(entry_to_info(&entry))
}

/// Reinstall a bot from the GitHub source declared in its existing
/// `manifest.toml`. Removes the current install first.
#[tauri::command]
pub async fn update_bot_from_manifest(
    name: String,
    state: State<'_, AppState>,
) -> CmdResult<BotInfo> {
    let dir = state.config.read().await.bot.dir.clone();
    let resolved = resolve_dir(Path::new(&dir));
    let registry = BotRegistry::scan(&resolved).map_err(|e| format!("scan bots: {e:#}"))?;
    let entry = registry
        .find(&name)
        .ok_or_else(|| format!("bot {name:?} not found"))?;
    let manifest = entry
        .manifest
        .as_ref()
        .ok_or_else(|| format!("bot {name:?} has no manifest.toml"))?;
    let source = manifest
        .source
        .as_ref()
        .ok_or_else(|| format!("bot {name:?} manifest has no [bot.source] block"))?;

    let (repo, asset_glob) = match source {
        BotSource::GithubRelease { repo, asset_glob } => (repo.clone(), asset_glob.clone()),
    };

    std::fs::remove_dir_all(&entry.dir)
        .map_err(|e| format!("remove old install {}: {e}", entry.dir.display()))?;

    let spec = GithubInstallSpec {
        repo,
        asset_glob,
        name: Some(name.clone()),
    };
    let new_entry = install::install_from_github_release(
        spec,
        &resolved,
        &state.notify_bus,
        state.runtime.as_ref(),
    )
    .await
    .map_err(|e| format!("install: {e:#}"))?;
    Ok(entry_to_info(&new_entry))
}

/// Re-run `uv sync` for an installed bot. Frontend wires this to the
/// "Reinstall environment" button under Configure. `force=true` wipes
/// `.akagi/synced.stamp` and `.akagi/venv` first so a corrupted venv is
/// rebuilt from scratch (incremental sync can otherwise mask the breakage).
/// Reports progress + outcome through `NotifyBus` with sticky id
/// `bot-sync-<name>`. Refuses to start a second concurrent sync for the
/// same bot.
#[tauri::command]
pub async fn sync_bot_deps(
    name: String,
    force: bool,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    let dir = state.config.read().await.bot.dir.clone();
    let resolved = resolve_dir(Path::new(&dir));
    let registry = BotRegistry::scan(&resolved).map_err(|e| format!("scan bots: {e:#}"))?;
    let entry = registry
        .find(&name)
        .ok_or_else(|| format!("bot {name:?} not found"))?
        .clone();
    let runtime = state.runtime.as_ref().ok_or_else(|| {
        "Python runtime not available — install python3 and uv on PATH".to_string()
    })?;

    let _guard = SyncGuard::acquire(&state.syncs_in_flight, &name)
        .await
        .ok_or_else(|| format!("sync already in progress for {name}"))?;

    let notify_id = format!("bot-sync-{name}");
    let _ = state.notify_bus.send(
        Notification::info(format!("Syncing {name}"))
            .body("Rebuilding Python environment (uv sync)…")
            .sticky()
            .id(notify_id.clone()),
    );

    if force {
        runtime::reset_sync_state(&entry.dir).await;
    }

    match runtime.ensure_synced(&entry.dir).await {
        Ok(()) => {
            let _ = state.notify_bus.send(
                Notification::success(format!("{name} environment ready")).id(notify_id),
            );
            Ok(())
        }
        Err(e) => {
            let msg = format!("uv sync failed: {e:#}");
            let _ = state.notify_bus.send(
                Notification::error(format!("Sync failed for {name}"))
                    .body(msg.clone())
                    .id(notify_id),
            );
            Err(msg)
        }
    }
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
    let (stop, force_close) = {
        let mut ctl = state.proxy_control.lock().await;
        (ctl.stop.take(), ctl.force_close.clone())
    };
    // Kick in-flight WS flows first so the game client actually
    // disconnects. Without this, hudsucker's graceful shutdown only
    // blocks new connections; existing ones drain naturally and the
    // user sees comm "still working" even after stop.
    force_close.notify_waiters();
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

/// Remove a bot's directory under `bot.dir/<name>/`. Refuses to delete
/// the currently-active bot — user must `set_active_bot` to a different
/// one first. Refuses target paths that escape `bot.dir` (defense in
/// depth even though `name` came from the bot list, not raw user input).
#[tauri::command]
pub async fn delete_bot(name: String, state: State<'_, AppState>) -> CmdResult<()> {
    let (active_4p, active_3p, dir) = {
        let cfg = state.config.read().await;
        (
            cfg.bot.active_4p.clone(),
            cfg.bot.active_3p.clone(),
            cfg.bot.dir.clone(),
        )
    };
    if active_4p == name || active_3p == name {
        return Err(format!(
            "{name:?} is an active bot (4p or 3p) — switch to a different bot first"
        ));
    }
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("invalid bot name {name:?}"));
    }
    let resolved_root = resolve_dir(Path::new(&dir));
    let target = resolved_root.join(&name);
    if !target.is_dir() {
        return Err(format!("bot {name:?} not found at {}", target.display()));
    }
    let canon_root = std::fs::canonicalize(&resolved_root)
        .map_err(|e| format!("canonicalize {}: {e}", resolved_root.display()))?;
    let canon_target = std::fs::canonicalize(&target)
        .map_err(|e| format!("canonicalize {}: {e}", target.display()))?;
    if !canon_target.starts_with(&canon_root) {
        return Err(format!("bot {name:?} resolves outside the bot directory"));
    }
    std::fs::remove_dir_all(&canon_target)
        .map_err(|e| format!("remove {}: {e}", canon_target.display()))?;
    let _ = state
        .notify_bus
        .send(Notification::success(format!("Deleted bot {name}")));
    Ok(())
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
            $crate::ipc::commands::get_bot_settings,
            $crate::ipc::commands::update_bot_settings,
            $crate::ipc::commands::install_bot_from_github,
            $crate::ipc::commands::update_bot_from_manifest,
            $crate::ipc::commands::sync_bot_deps,
            $crate::ipc::commands::delete_bot,
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
        cfg.bot.active_4p = "mortal".into();
        cfg.bot.active_3p = "mortal_3p".into();
        cfg.proxy.addr = "127.0.0.1:9999".into();

        persist_config(&cfg, &path).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        let back: AppConfig = toml::from_str(&body).unwrap();
        assert_eq!(back.bot.active_4p, "mortal");
        assert_eq!(back.bot.active_3p, "mortal_3p");
        assert_eq!(back.proxy.addr, "127.0.0.1:9999");
    }
}
