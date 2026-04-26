//! Public types exchanged between `BotRunner` and the rest of Akagi.
//!
//! Wire shape matches the mjai.app convention: an mjai action object with an
//! optional `meta` sibling for HUD-grade recommendation data.
//!
//! Example wire JSON:
//! ```text
//! {"type":"dahai","actor":0,"pai":"9m","tsumogiri":false,
//!  "meta":{"q_values":[0.12,0.05,0.85],"is_greedy":true}}
//! ```

use crate::schema::MjaiEvent;
use serde::{Deserialize, Serialize};

/// One reaction from a `BotRunner`.
///
/// `action` is uniformly an `MjaiEvent` — `MjaiEvent::None` means "no
/// action this turn" (see `src/schema/mjai/mod.rs`). Consumers can match
/// on the variant to decide whether to render or skip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotResponse {
    #[serde(flatten)]
    pub action: MjaiEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<BotMeta>,
}

/// Optional recommendation metadata produced by NN-based bots.
///
/// All fields are optional so simple bots (shanten/tsumogiri) can omit
/// them without writing nulls. NN bots like Mortal populate `q_values`
/// + `mask_bits` so the HUD can rank discards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q_values: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_bits: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_greedy: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_action_round_trips() {
        let line = r#"{"type":"none"}"#;
        let resp: BotResponse = serde_json::from_str(line).unwrap();
        assert!(matches!(resp.action, MjaiEvent::None));
        assert!(resp.meta.is_none());
        assert_eq!(serde_json::to_string(&resp).unwrap(), line);
    }

    #[test]
    fn dahai_with_meta_round_trips() {
        let line = r#"{"type":"dahai","actor":0,"pai":"9m","tsumogiri":false,"meta":{"q_values":[0.12,0.05,0.85],"is_greedy":true}}"#;
        let resp: BotResponse = serde_json::from_str(line).unwrap();
        match &resp.action {
            MjaiEvent::Dahai { actor, pai, tsumogiri } => {
                assert_eq!(*actor, 0);
                assert_eq!(pai, "9m");
                assert!(!tsumogiri);
            }
            other => panic!("expected dahai, got {other:?}"),
        }
        let meta = resp.meta.as_ref().unwrap();
        assert_eq!(meta.q_values.as_deref(), Some(&[0.12, 0.05, 0.85][..]));
        assert_eq!(meta.is_greedy, Some(true));

        // Round-trip equivalence (key order is structural, not lexical).
        let v1: serde_json::Value = serde_json::from_str(line).unwrap();
        let v2: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn dahai_without_meta_skips_field() {
        let resp = BotResponse {
            action: MjaiEvent::Dahai {
                actor: 1,
                pai: "5mr".into(),
                tsumogiri: true,
            },
            meta: None,
        };
        let out = serde_json::to_string(&resp).unwrap();
        assert!(!out.contains("meta"), "meta:None should not be emitted: {out}");
    }
}
