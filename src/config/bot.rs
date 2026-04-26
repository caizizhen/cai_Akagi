use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BotConfig {
    /// Master switch. When `false`, no `BotManager` is spawned and the
    /// MJAI event bus runs without a consumer.
    pub enabled: bool,
    /// Subdirectory of `dir` that holds the active bot's `bot.py`.
    pub active: String,
    /// Run `uv sync` automatically before spawning the bot. Disabling
    /// makes startup faster on slow disks but assumes the venv is
    /// already in sync — usually for advanced users.
    pub auto_sync: bool,
    /// Root directory containing one subdir per bot. Resolved with the
    /// same fallback chain as other directory configs (`util::resolve_dir`).
    pub dir: String,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            active: "example".to_string(),
            auto_sync: true,
            dir: "mjai_bot".to_string(),
        }
    }
}
