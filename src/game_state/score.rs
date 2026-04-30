//! Thin wrappers over `riichienv_core::score` and `HandEvaluator`.
//!
//! The intent is to give the rest of Akagi (and eventually IPC commands)
//! a stable surface that doesn't leak the upstream crate's exact types.
//! When riichienv tweaks its API in a future bump, this is the only
//! file that has to change.

use anyhow::{Context, Result};
use riichienv_core::hand_evaluator::HandEvaluator;
use riichienv_core::parser::tid_to_mjai;
pub use riichienv_core::score::Score;
use riichienv_core::state::GameState;
use riichienv_core::state_3p::GameState3P;
use riichienv_core::types::{Conditions, Wind};

use crate::schema::HoraScoreInfo;

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

/// Evaluate a 4-player ron/tsumo claim by `actor` against the live engine
/// state. Returns `None` when the actor's hand isn't a winning shape, when
/// the winning tile can't be inferred from `state` (no recent
/// discard / draw), or when `actor` is out of bounds.
///
/// The actor's hand is taken from `state.players[actor].hand` (136-space),
/// melds from `state.players[actor].melds`. The winning tile is
/// `state.last_discard.0` for ron and `state.drawn_tile` for tsumo. Dora
/// indicators come from `state.wall.dora_indicators`; ura dora is left
/// empty (only revealed after the platform accepts the win).
///
/// `Conditions` mirrors how `state/mod.rs` builds them on real wins:
/// haitei/houtei from `wall.drawable_count` + `is_rinshan_flag`,
/// rinshan from `is_rinshan_flag`, ippatsu from `players[actor].ippatsu_cycle`,
/// chankan from `pending_kan.is_some()` (ron only).
pub fn evaluate_hora_4p(state: &GameState, actor: u8, is_tsumo: bool) -> Option<HoraScoreInfo> {
    let actor_idx = actor as usize;
    if actor_idx >= state.players.len() {
        return None;
    }
    let player = &state.players[actor_idx];

    let win_tile_136 = if is_tsumo {
        state.drawn_tile?
    } else {
        state.last_discard.map(|(t, _)| t)?
    };

    let evaluator = HandEvaluator::new(player.hand.clone(), player.melds.clone());

    let conditions = Conditions {
        tsumo: is_tsumo,
        riichi: player.riichi_declared,
        double_riichi: player.double_riichi_declared,
        ippatsu: player.ippatsu_cycle,
        haitei: is_tsumo && state.wall.drawable_count == 0 && !state.is_rinshan_flag,
        houtei: !is_tsumo && state.wall.drawable_count == 0 && !state.is_rinshan_flag,
        rinshan: is_tsumo && state.is_rinshan_flag,
        chankan: !is_tsumo && state.pending_kan.is_some(),
        tsumo_first_turn: is_tsumo && state.is_first_turn,
        player_wind: Wind::from((actor + 4 - state.oya) % 4),
        round_wind: Wind::from(state.round_wind),
        riichi_sticks: state.riichi_sticks,
        honba: state.honba as u32,
        kita_count: 0,
        is_sanma: false,
        num_players: 4,
    };

    let result = evaluator.calc(
        win_tile_136,
        state.wall.dora_indicators.clone(),
        Vec::new(),
        Some(conditions),
    );
    if !result.is_win {
        return None;
    }
    let points = if is_tsumo {
        let oya_seat = state.oya;
        if actor == oya_seat {
            // Dealer tsumo: each of 3 ko pays `tsumo_agari_ko`.
            result.tsumo_agari_ko.saturating_mul(3)
        } else {
            // Non-dealer tsumo: 1 oya pay + 2 ko pays.
            result
                .tsumo_agari_oya
                .saturating_add(result.tsumo_agari_ko.saturating_mul(2))
        }
    } else {
        result.ron_agari
    };
    Some(HoraScoreInfo {
        points,
        han: result.han,
        fu: result.fu,
        yakuman: result.yakuman,
        win_tile: tid_to_mjai(win_tile_136),
    })
}

/// 3-player variant. Tsumo splits across 2 ko (or 2 ko for dealer in 3p).
pub fn evaluate_hora_3p(state: &GameState3P, actor: u8, is_tsumo: bool) -> Option<HoraScoreInfo> {
    let actor_idx = actor as usize;
    if actor_idx >= state.players.len() {
        return None;
    }
    let player = &state.players[actor_idx];

    let win_tile_136 = if is_tsumo {
        state.drawn_tile?
    } else {
        state.last_discard.map(|(t, _)| t)?
    };

    let evaluator = HandEvaluator::new(player.hand.clone(), player.melds.clone());

    let conditions = Conditions {
        tsumo: is_tsumo,
        riichi: player.riichi_declared,
        double_riichi: player.double_riichi_declared,
        ippatsu: player.ippatsu_cycle,
        haitei: is_tsumo && state.wall.drawable_count == 0 && !state.is_rinshan_flag,
        houtei: !is_tsumo && state.wall.drawable_count == 0 && !state.is_rinshan_flag,
        rinshan: is_tsumo && state.is_rinshan_flag,
        chankan: !is_tsumo && state.pending_kan.is_some(),
        tsumo_first_turn: is_tsumo && state.is_first_turn,
        player_wind: Wind::from((actor + 3 - state.oya) % 3),
        round_wind: Wind::from(state.round_wind),
        riichi_sticks: state.riichi_sticks,
        honba: state.honba as u32,
        kita_count: player.kita_tiles.len() as u8,
        is_sanma: true,
        num_players: 3,
    };

    let result = evaluator.calc(
        win_tile_136,
        state.wall.dora_indicators.clone(),
        Vec::new(),
        Some(conditions),
    );
    if !result.is_win {
        return None;
    }
    let points = if is_tsumo {
        let oya_seat = state.oya;
        if actor == oya_seat {
            // Dealer tsumo (3p): 2 ko pay.
            result.tsumo_agari_ko.saturating_mul(2)
        } else {
            // Non-dealer tsumo (3p): 1 oya pay + 1 ko pay.
            result
                .tsumo_agari_oya
                .saturating_add(result.tsumo_agari_ko)
        }
    } else {
        result.ron_agari
    };
    Some(HoraScoreInfo {
        points,
        han: result.han,
        fu: result.fu,
        yakuman: result.yakuman,
        win_tile: tid_to_mjai(win_tile_136),
    })
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

    /// Build a closed-hand GameState where seat 0 is in chiitoitsu tenpai
    /// for `7s`. Used by the `evaluate_hora_4p_*` tests below — we set
    /// `drawn_tile` (tsumo) or `last_discard` (ron) to 7s after running
    /// `start_kyoku`-equivalent setup.
    ///
    /// Tile IDs are in 136-space; we pick the lowest copy of each tile to
    /// keep things deterministic and avoid red-five sentinels (16/52/88).
    fn chiitoitsu_state(actor: u8, oya: u8) -> GameState {
        let rule = riichienv_core::rule::GameRule::default_tenhou();
        let mut s = GameState::new(0, true, None, 0, rule);
        s.oya = oya;
        s.round_wind = 0; // East
        s.honba = 0;
        s.riichi_sticks = 0;
        s.is_first_turn = false;

        // 11m 22m 33p 44p 55s 66s + tenpai-on-7s (13 tiles).
        let hand = vec![
            0, 1, // 1m 1m
            4, 5, // 2m 2m
            44, 45, // 3p 3p
            48, 49, // 4p 4p
            92, 93, // 5s 5s
            96, 97, // 6s 6s
            100, // 7s
        ];
        s.players[actor as usize].hand = hand;
        s
    }

    #[test]
    fn evaluate_hora_4p_tsumo_chiitoitsu_dealer() {
        // Dealer (oya=0) tsumo on 7s, chiitoitsu shape. Closed hand →
        // chiitoitsu (2 han) + menzen tsumo (1 han) at the very least.
        // Score must be a positive multiple of 100 with han ≥ 1.
        let mut s = chiitoitsu_state(0, 0);
        s.drawn_tile = Some(101); // 7s (second copy)
        s.players[0].hand.push(101);

        let info = evaluate_hora_4p(&s, 0, true).expect("winning shape");
        assert!(info.han >= 1, "han = {}", info.han);
        assert!(info.points > 0, "points = {}", info.points);
        assert!(info.points % 100 == 0, "points = {}", info.points);
    }

    #[test]
    fn evaluate_hora_4p_ron_chiitoitsu_non_dealer() {
        // Non-dealer (oya=1) ron on 7s discarded by seat 1. Chiitoitsu → 2 han.
        let mut s = chiitoitsu_state(0, 1);
        s.last_discard = Some((101, 1)); // 7s, discarder = seat 1

        let info = evaluate_hora_4p(&s, 0, false).expect("winning shape");
        assert!(info.han >= 2, "han = {}", info.han);
        assert!(info.fu >= 20, "fu = {}", info.fu);
        assert!(info.points > 0, "points = {}", info.points);
    }

    #[test]
    fn evaluate_hora_4p_returns_none_when_not_a_win() {
        // Default GameState has empty hands → not a winning shape.
        let rule = riichienv_core::rule::GameRule::default_tenhou();
        let mut s = GameState::new(0, true, None, 0, rule);
        s.drawn_tile = Some(0);
        assert!(evaluate_hora_4p(&s, 0, true).is_none());
    }

    #[test]
    fn evaluate_hora_4p_returns_none_when_no_win_tile() {
        // Winning shape in hand but `drawn_tile` is None and `last_discard`
        // is None — we can't infer the winning tile.
        let s = chiitoitsu_state(0, 0);
        assert!(evaluate_hora_4p(&s, 0, true).is_none());
        assert!(evaluate_hora_4p(&s, 0, false).is_none());
    }

    #[test]
    fn evaluate_hora_4p_returns_none_for_oob_actor() {
        let mut s = chiitoitsu_state(0, 0);
        s.drawn_tile = Some(101);
        assert!(evaluate_hora_4p(&s, 99, true).is_none());
    }

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
