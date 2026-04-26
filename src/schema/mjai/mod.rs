//! mjai JSONL protocol types.
//!
//! See `reference/reference_mjai.md` for the spec, and
//! `reference/Mortal/libriichi/src/mjai/event.rs` for a richer reference impl
//! (typed `Tile`, bounded actor, augmentation, metadata).
//!
//! This impl keeps tiles as `String` so the bridge can stay decoupled from any
//! tile-encoding library. JSON output puts `"type"` first because
//! `#[serde(tag = "type")]` (internally-tagged enum) emits the tag field first.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// mjai tile string. Examples: `"1m"`, `"5mr"` (red 5), `"E"`, `"P"`, `"?"`.
pub type Tile = String;

/// Seat index, 0..=3 (4p) or 0..=2 (3p).
pub type Actor = u8;

/// One mjai event. Serialized as `{"type":"<snake_case>", ...}` with `type` first.
///
/// Mirrors the 15 event types in `reference/reference_mjai.md`.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MjaiEvent {
    StartGame {
        names: [String; 4],
        kyoku_first: Option<u8>,
        aka_flag: Option<bool>,
        /// Bot's own seat (0..=3). Mjai.app convention: when this start_game
        /// is the *bot's view*, `id` tells the bot which seat it plays.
        /// Omitted on neutral/server-side logs.
        id: Option<Actor>,
    },
    StartKyoku {
        bakaze: Tile,
        dora_marker: Tile,
        /// 1..=4
        kyoku: u8,
        honba: u8,
        kyotaku: u8,
        oya: Actor,
        scores: [i32; 4],
        tehais: [[Tile; 13]; 4],
    },

    Tsumo {
        actor: Actor,
        pai: Tile,
    },
    Dahai {
        actor: Actor,
        pai: Tile,
        tsumogiri: bool,
    },

    Chi {
        actor: Actor,
        target: Actor,
        pai: Tile,
        consumed: [Tile; 2],
    },
    Pon {
        actor: Actor,
        target: Actor,
        pai: Tile,
        consumed: [Tile; 2],
    },
    Daiminkan {
        actor: Actor,
        target: Actor,
        pai: Tile,
        consumed: [Tile; 3],
    },
    Kakan {
        actor: Actor,
        pai: Tile,
        consumed: [Tile; 3],
    },
    Ankan {
        actor: Actor,
        consumed: [Tile; 4],
    },
    Dora {
        dora_marker: Tile,
    },

    Reach {
        actor: Actor,
    },
    ReachAccepted {
        actor: Actor,
    },

    Hora {
        actor: Actor,
        target: Actor,
        deltas: Option<[i32; 4]>,
        ura_markers: Option<Vec<Tile>>,
    },
    Ryukyoku {
        deltas: Option<[i32; 4]>,
    },

    EndKyoku,
    EndGame,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{self, Value};

    /// Round-trip every event variant through JSON, asserting `type` is the
    /// first key in the serialized output and the value matches the spec.
    #[test]
    fn json_consistency_and_type_first() {
        let lines = [
            r#"{"type":"start_game","names":["NIKUYA","ひぐち","龍10","cxytml"],"kyoku_first":0,"aka_flag":true,"id":2}"#,
            r#"{"type":"start_game","names":["NIKUYA","ひぐち","龍10","cxytml"],"kyoku_first":0,"aka_flag":true}"#,
            r#"{"type":"start_kyoku","bakaze":"E","dora_marker":"2m","kyoku":1,"honba":0,"kyotaku":0,"oya":0,"scores":[25000,25000,25000,25000],"tehais":[["5mr","9m","3p","1s","4s","4s","5sr","6s","6s","7s","E","E","P"],["4m","4m","6m","7m","8m","3p","5p","5pr","2s","9s","S","S","C"],["1m","4p","4p","5p","7p","8p","2s","3s","3s","5s","7s","7s","F"],["1m","1m","4m","5m","1p","2p","2s","6s","8s","W","N","F","C"]]}"#,
            r#"{"type":"tsumo","actor":0,"pai":"C"}"#,
            r#"{"type":"dahai","actor":0,"pai":"9m","tsumogiri":false}"#,
            r#"{"type":"chi","actor":3,"target":2,"pai":"3m","consumed":["4m","5mr"]}"#,
            r#"{"type":"pon","actor":1,"target":0,"pai":"4m","consumed":["4m","4m"]}"#,
            r#"{"type":"kakan","actor":0,"pai":"6m","consumed":["6m","6m","6m"]}"#,
            r#"{"type":"ankan","actor":0,"consumed":["F","F","F","F"]}"#,
            r#"{"type":"daiminkan","actor":0,"target":2,"pai":"5m","consumed":["5m","5m","5mr"]}"#,
            r#"{"type":"dora","dora_marker":"3s"}"#,
            r#"{"type":"reach","actor":0}"#,
            r#"{"type":"reach_accepted","actor":0}"#,
            r#"{"type":"hora","actor":2,"target":3,"deltas":[0,0,2300,-1300],"ura_markers":["C"]}"#,
            r#"{"type":"hora","actor":2,"target":3}"#,
            r#"{"type":"ryukyoku","deltas":[0,1500,0,-1500]}"#,
            r#"{"type":"ryukyoku"}"#,
            r#"{"type":"end_kyoku"}"#,
            r#"{"type":"end_game"}"#,
        ];

        for line in lines {
            let event: MjaiEvent = serde_json::from_str(line).expect("deserialize");
            let out = serde_json::to_string(&event).expect("serialize");

            assert!(
                out.starts_with(r#"{"type":""#),
                "type must be first key, got: {out}"
            );

            let expected: Value = serde_json::from_str(line).unwrap();
            let actual: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(expected, actual, "round-trip mismatch for {line}");
        }
    }
}
