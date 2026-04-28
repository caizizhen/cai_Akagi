//! Backend ↔ frontend IPC payload types.
//!
//! These types travel two directions:
//!
//! - **Backend → frontend**: emitted as Tauri events by `crate::ipc`. Names
//!   match the Tauri event names (kebab-case): `notify`, `bot-status`,
//!   `proxy-status`.
//! - **Frontend → backend**: returned from `#[tauri::command]` handlers in
//!   `crate::ipc::commands`.
//!
//! All types are `Serialize + Deserialize` so they round-trip through Tauri's
//! JSON bridge and through the in-process broadcast buses in
//! `crate::event_bus`. Variants are tagged so the wire shape stays stable
//! when new states are added — frontends can match on `state` / `level` /
//! `stage` without positional surprises.

use serde::{Deserialize, Serialize};

// ---------- Notification ----------

/// Severity for a frontend toast / notification box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    Info,
    Success,
    Warn,
    Error,
}

/// One frontend-facing notification. Any subsystem may push these onto
/// `event_bus::NotifyBus`; `crate::ipc` forwards them to all webviews as
/// the `notify` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub level: NotifyLevel,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `true` → frontend keeps the toast until the user dismisses it.
    /// Used for long-running blocking states (e.g. "installing bot deps").
    #[serde(default)]
    pub sticky: bool,
    /// Stable key. Frontends should replace any prior toast with the same
    /// `id`, so progress updates don't pile up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl Notification {
    pub fn info(title: impl Into<String>) -> Self {
        Self::new(NotifyLevel::Info, title)
    }
    pub fn success(title: impl Into<String>) -> Self {
        Self::new(NotifyLevel::Success, title)
    }
    pub fn warn(title: impl Into<String>) -> Self {
        Self::new(NotifyLevel::Warn, title)
    }
    pub fn error(title: impl Into<String>) -> Self {
        Self::new(NotifyLevel::Error, title)
    }

    fn new(level: NotifyLevel, title: impl Into<String>) -> Self {
        Self {
            level,
            title: title.into(),
            body: None,
            sticky: false,
            id: None,
        }
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn sticky(mut self) -> Self {
        self.sticky = true;
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

// ---------- BotStatus ----------

/// Sub-stage during the slow first-spawn path.
///
/// `SyncingDeps` is the long one — `uv sync` downloads every wheel listed
/// in the bot's `pyproject.toml` into `<bot_dir>/.akagi/venv` on first use,
/// which can take tens of seconds. `Spawning` is the short tail (process
/// creation + waiting for the first react round-trip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadStage {
    SyncingDeps,
    Spawning,
}

/// Lifecycle state of the active bot subprocess.
///
/// Tagged with `state` so the frontend can match on a string discriminant
/// (`{"state":"loading","bot":"mortal","stage":"syncing_deps"}`) without
/// having to know Rust's untagged-enum trick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BotStatus {
    /// No active bot — waiting for `start_game` (or bot disabled).
    Idle,
    /// First-time install / spawn in progress. UI should show a spinner.
    Loading { bot: String, stage: LoadStage },
    /// Subprocess up and reacting.
    Ready { bot: String, actor_id: u8 },
    /// Spawn or react path failed; runner torn down.
    Error { bot: String, error: String },
    /// Game ended (`EndGame`) — runner dropped, awaiting next start.
    Stopped { bot: String },
}

// ---------- ProxyStatus ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProxyStatus {
    Stopped,
    Starting { addr: String },
    Running { addr: String },
    Error { addr: Option<String>, error: String },
}

// ---------- Snapshot / BotInfo ----------
//
// Returned by `commands::get_status` and `commands::list_bots`. These
// types pull in `AppConfig` (one direction only — schema doesn't cycle
// back into config). Keep these stable; the React side keys off field
// names.

use crate::config::AppConfig;
use std::path::PathBuf;

/// One discovered bot, IPC-shaped (separate from `bot::BotEntry` so the
/// wire contract can evolve independently of the registry internals).
///
/// `manifest` carries the bot's settings schema (rendered as a form on
/// the frontend). Bots without a `manifest.toml` have it set to `None`
/// and the UI hides the settings panel for them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotInfo {
    pub name: String,
    pub dir: String,
    pub has_pyproject: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<crate::bot::manifest::Manifest>,
}

/// Returned by the `get_bot_settings` command. `values` is the result of
/// merging defaults from `manifest` with whatever lives in the bot's
/// `settings.toml`. Frontend should treat `manifest.settings` as the form
/// schema and `values` as the form's current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotSettings {
    pub manifest: crate::bot::manifest::Manifest,
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
}

/// One-shot dump of everything the UI needs on startup. Cheap to clone
/// (config + two enums + a path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub config: AppConfig,
    pub bot_status: BotStatus,
    pub proxy_status: ProxyStatus,
    pub log_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_minimal_round_trips() {
        let n = Notification::info("hi");
        let s = serde_json::to_string(&n).unwrap();
        // No body / id; sticky=false is emitted (no skip_if).
        assert_eq!(s, r#"{"level":"info","title":"hi","sticky":false}"#);
        let back: Notification = serde_json::from_str(&s).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn notification_full_round_trips() {
        let n = Notification::error("boom")
            .body("stack trace here")
            .sticky()
            .id("bot-load-fail");
        let s = serde_json::to_string(&n).unwrap();
        let back: Notification = serde_json::from_str(&s).unwrap();
        assert_eq!(back, n);
        assert!(s.contains(r#""level":"error""#));
        assert!(s.contains(r#""sticky":true"#));
        assert!(s.contains(r#""id":"bot-load-fail""#));
    }

    #[test]
    fn bot_status_loading_tagged_correctly() {
        let s = BotStatus::Loading {
            bot: "mortal".into(),
            stage: LoadStage::SyncingDeps,
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(
            j,
            r#"{"state":"loading","bot":"mortal","stage":"syncing_deps"}"#
        );
        let back: BotStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn bot_status_ready_round_trips() {
        let s = BotStatus::Ready {
            bot: "example".into(),
            actor_id: 2,
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(
            j,
            r#"{"state":"ready","bot":"example","actor_id":2}"#
        );
        let back: BotStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn bot_status_idle_has_no_extra_fields() {
        let s = BotStatus::Idle;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, r#"{"state":"idle"}"#);
    }

    #[test]
    fn proxy_status_round_trips() {
        let s = ProxyStatus::Running {
            addr: "127.0.0.1:23410".into(),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: ProxyStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn proxy_status_error_with_no_addr() {
        let s = ProxyStatus::Error {
            addr: None,
            error: "bind failed".into(),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains(r#""state":"error""#));
        assert!(j.contains(r#""addr":null"#));
    }
}
