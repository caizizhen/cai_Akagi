//! Shared schema types used across the project.
//!
//! Anything that needs to travel between modules — protocol events, IPC
//! payloads between backend and frontend, persisted records — lives here so
//! it isn't owned by any single subsystem.

pub mod ipc;
pub mod mjai;

pub use ipc::{
    BotInfo, BotSettings, BotStatus, HoraScoreInfo, LoadStage, NotifyLevel, Notification,
    ProxyStatus, Snapshot,
};
pub use mjai::MjaiEvent;
