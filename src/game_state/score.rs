//! Thin wrappers over `riichienv_core::score` and `HandEvaluator`.
//!
//! The intent is to give the rest of Akagi (and eventually IPC commands)
//! a stable surface that doesn't leak the upstream crate's exact types.
//! When riichienv tweaks its API in a future bump, this is the only
//! file that has to change.

use anyhow::{Context, Result};
use riichienv_core::hand_evaluator::HandEvaluator;
pub use riichienv_core::score::Score;

/// Map a tile index in 34-space (0..34) to the mjai string. Unlike
/// `riichienv_core::parser::tid_to_mjai` (which works in 136-space and
/// turns the red-five sentinels 16/52/88 into `"5mr"`), this never
/// emits red-five suffixes — `get_waits_u8` returns 34-space indices
/// where reds aren't distinguished.
fn tid34_to_mjai(idx: u8) -> String {
    const HONORS: [&str; 7] = ["E", "S", "W", "N", "P", "F", "C"];
    if idx < 27 {
        let suit = ["m", "p", "s"][(idx / 9) as usize];
        format!("{}{}", (idx % 9) + 1, suit)
    } else {
        HONORS[(idx - 27) as usize].to_string()
    }
}

/// Calculate hand score for a given (han, fu, oya?, tsumo?, honba, num_players) tuple.
///
/// `num_players` is 3 for sanma or 4 for yonma. Affects honba payment splits
/// and (in 3p) tsumo payer count.
pub fn calculate_score(
    han: u8,
    fu: u8,
    is_oya: bool,
    is_tsumo: bool,
    honba: u32,
    num_players: u8,
) -> Score {
    riichienv_core::score::calculate_score(han, fu, is_oya, is_tsumo, honba, num_players)
}

/// Tenpai check + waits for a hand string in `riichienv` MPSZ notation
/// (e.g. `"123m456p789s1122z"`, `"(p123m)456p789s1122z"` for melds).
///
/// Returns the wait list as mjai tile strings (`"1m"`, `"E"`, `"5pr"`),
/// or `Ok(vec![])` if the hand is not tenpai.
pub fn waits_for(hand_text: &str) -> Result<Vec<String>> {
    let evaluator = HandEvaluator::hand_from_text(hand_text)
        .with_context(|| format!("parse hand: {hand_text:?}"))?;
    if !evaluator.is_tenpai() {
        return Ok(Vec::new());
    }
    Ok(evaluator
        .get_waits_u8()
        .into_iter()
        .map(tid34_to_mjai)
        .collect())
}

/// Tenpai check only, when the wait list isn't needed.
pub fn is_tenpai(hand_text: &str) -> Result<bool> {
    let evaluator = HandEvaluator::hand_from_text(hand_text)
        .with_context(|| format!("parse hand: {hand_text:?}"))?;
    Ok(evaluator.is_tenpai())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ron_3han_30fu_non_dealer_no_honba() {
        let s = calculate_score(3, 30, false, false, 0, 4);
        // 30 fu × 2^(2+3) = 960 base; non-dealer ron = ceil_100(960*4) = 3900.
        assert_eq!(s.total, 3900);
        assert_eq!(s.pay_ron, 3900);
    }

    #[test]
    fn mangan_dealer_tsumo() {
        // 5 han = mangan; dealer tsumo = 4000 from each ko = 12000.
        let s = calculate_score(5, 30, true, true, 0, 4);
        assert_eq!(s.total, 12_000);
        assert_eq!(s.pay_tsumo_ko, 4_000);
    }

    #[test]
    fn honba_adds_300_to_ron_4p() {
        let base = calculate_score(2, 30, false, false, 0, 4);
        let with_honba = calculate_score(2, 30, false, false, 1, 4);
        // 4p ron honba = 300 added to ron pay + total.
        assert_eq!(with_honba.total, base.total + 300);
        assert_eq!(with_honba.pay_ron, base.pay_ron + 300);
    }

    #[test]
    fn waits_finds_chiitoitsu_tanki() {
        // 6 pairs + a lone 7m → chiitoitsu tenpai on 7m. Standard
        // interpretations also yield 1m and 4m via overlapping runs
        // (11+234+234+567+56 waits on 4m, etc.), so assert membership
        // rather than exact equality.
        let waits = waits_for("1122334455667m").unwrap();
        assert!(waits.contains(&"7m".into()), "got {waits:?}");
    }

    #[test]
    fn waits_returns_empty_for_non_tenpai() {
        // Random shanten ≥ 1 hand.
        let waits = waits_for("123456789m12345p").unwrap();
        assert!(waits.is_empty());
    }

    #[test]
    fn is_tenpai_consistent_with_waits() {
        let hand = "1122334455667m";
        assert!(is_tenpai(hand).unwrap());
        assert!(!waits_for(hand).unwrap().is_empty());
    }

    #[test]
    fn tid34_covers_all_suits_and_honors() {
        assert_eq!(tid34_to_mjai(0), "1m");
        assert_eq!(tid34_to_mjai(8), "9m");
        assert_eq!(tid34_to_mjai(9), "1p");
        assert_eq!(tid34_to_mjai(17), "9p");
        assert_eq!(tid34_to_mjai(18), "1s");
        assert_eq!(tid34_to_mjai(26), "9s");
        assert_eq!(tid34_to_mjai(27), "E");
        assert_eq!(tid34_to_mjai(33), "C");
    }
}
