//! Shared schema types used across the project.
//!
//! Anything that needs to travel between modules — protocol events, IPC
//! payloads between backend and frontend, persisted records — lives here so
//! it isn't owned by any single subsystem.

pub mod history;
pub mod ipc;
pub mod mjai;

pub use history::{
    GameRecord, GameStats, HistoryEvent, HistoryEventLog, HistoryFilter, KyokuMode, Platform,
};
pub use ipc::{
    BotInfo, BotSettings, BotStatus, CaptureKind, CaptureStatus, HoraScoreInfo, LoadStage,
    Notification, NotifyLevel, Snapshot,
};
pub use mjai::MjaiEvent;
