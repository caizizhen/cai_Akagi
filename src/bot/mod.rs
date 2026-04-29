//! AI bot integration.
//!
//! Wires `MjaiEvent`s from the platform bridge to AI bots speaking the
//! [mjai JSONL protocol](../../reference/reference_mjai.md). Each bot runs
//! in its own subprocess (Python, by default) so AGPL-licensed bots like
//! Mortal stay legally separated from Akagi's binary.
//!
//! See `claude_plan_bot_runner.md` for the full design and `README.md` in
//! this directory for the contributor-facing how-to.

pub mod install;
pub mod manager;
pub mod manifest;
pub mod registry;
pub mod runner;
pub mod runtime;
pub mod sync_guard;
pub mod types;

pub use install::{GithubInstallSpec, install_from_github_release};
pub use manager::BotManager;
pub use manifest::{BotSource, FieldKind, FieldSpec, Manifest, ManifestBot};
pub use registry::{BotEntry, BotRegistry};
pub use runner::{BotRunner, SubprocessBot};
pub use runtime::{PythonRuntime, RuntimeMode};
pub use sync_guard::SyncGuard;
pub use types::BotResponse;
