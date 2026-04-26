//! Platform-specific protocol bridges.
//!
//! A `Bridge` translates between a game platform's wire protocol and the
//! mjai JSONL event stream consumed by AI bots. One bridge instance per
//! independent game session (e.g. one Majsoul WebSocket flow).

pub mod majsoul;

pub use majsoul::MajsoulBridge;

use crate::{
    logger::{FlowLogger, Session},
    schema::MjaiEvent,
};
use std::sync::Arc;

/// Direction of a parsed frame relative to the proxied client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Client → server (uplink, e.g. requests).
    Up,
    /// Server → client (downlink, e.g. responses, notifies).
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

/// Translates raw platform frames to mjai events and vice-versa.
pub trait Bridge: Send {
    /// Parse a raw platform frame into zero or more mjai events.
    fn parse(&mut self, direction: Direction, content: &[u8]) -> Vec<MjaiEvent>;

    /// Build a raw platform frame from an mjai command, if applicable.
    fn build(&mut self, command: &MjaiEvent) -> Option<Vec<u8>>;
}

/// Construct a bridge for the given platform.
///
/// - `flow_log`: per-WS-flow text dump (one JSON line per parsed message).
/// - `session`: passed through to bridges that open additional log files
///   on demand (e.g. Majsoul rotates a fresh `*.mjai.jsonl` per game).
pub fn for_platform(
    platform: crate::config::Platform,
    flow_log: Option<Arc<FlowLogger>>,
    session: Option<Arc<Session>>,
) -> Box<dyn Bridge> {
    match platform {
        crate::config::Platform::Majsoul => Box::new(MajsoulBridge::new(flow_log, session)),
    }
}
