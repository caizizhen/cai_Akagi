//! In-process broadcast buses connecting Akagi's subsystems.
//!
//! Two buses, both `tokio::sync::broadcast::Sender`-typed:
//!
//! - [`MjaiBus`]: every `MjaiEvent` parsed by a platform bridge is fanned
//!   out here. Producers: bridge → proxy handler. Consumers: `BotManager`,
//!   future HUD/storage/WS server.
//! - [`BotResponseBus`]: every `BotResponse` from the active `BotRunner`.
//!   Producer: `BotManager`. Consumers: future HUD / external WS / replay
//!   recorder.
//!
//! Channel capacity is fixed-size — slow consumers see `RecvError::Lagged`
//! rather than blocking the producer. That's the right trade-off for a
//! real-time analyzer: if the HUD falls behind, drop and resync rather
//! than stall the proxy.

use crate::bot::BotResponse;
use crate::schema::MjaiEvent;
use tokio::sync::broadcast;

/// Fan-out for `MjaiEvent`s from platform bridges.
pub type MjaiBus = broadcast::Sender<MjaiEvent>;

/// Fan-out for `BotResponse`s from the active bot.
pub type BotResponseBus = broadcast::Sender<BotResponse>;

/// Default capacity. ~1 second of mjai events at peak game pace
/// (start_kyoku + 13 tehai + many tsumo/dahai pairs) is well under 256.
pub const DEFAULT_CAPACITY: usize = 256;

pub fn mjai_bus() -> MjaiBus {
    // Drop the placeholder receiver — real consumers subscribe later via
    // `Sender::subscribe`. The sender stays alive as long as anyone holds
    // a clone of it.
    let (tx, _rx) = broadcast::channel(DEFAULT_CAPACITY);
    tx
}

pub fn bot_response_bus() -> BotResponseBus {
    let (tx, _rx) = broadcast::channel(DEFAULT_CAPACITY);
    tx
}
