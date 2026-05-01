//! Per-flow Tenhou game state mirror.
//!
//! Tenhou messages all use *relative* seating, where seat 0 is always the
//! observing player. We resolve the absolute seat from the `<TAIKYOKU/>` tag
//! (`oya` is dealer's relative seat) and translate every subsequent reference
//! through [`State::rel_to_abs`].

use super::meld::Meld;

#[derive(Debug, Clone)]
pub struct State {
    /// Our player's absolute seat (0..=3 for yonma, 0..=2 for sanma).
    pub seat: u8,
    /// Tenhou tile indices in our hand (other players' hands are not tracked).
    pub hand: Vec<u32>,
    /// Open melds we have called this kyoku.
    pub melds: Vec<Meld>,
    pub in_riichi: bool,
    /// Tiles remaining in the wall (counts down from 70 each kyoku).
    pub live_wall: u32,
    /// Last discard, stored as mjai string for ron-target attribution.
    pub last_kawa_tile: String,
    /// True between our tsumo and our dahai.
    pub is_tsumo: bool,
    /// True iff this is a 3-player (sanma) game.
    pub is_3p: bool,
    /// 3 (sanma) or 4 (yonma).
    pub num_players: u8,
    /// Absolute seat that performed the most recent revealing action
    /// (discard / kakan / ankan), so a subsequent `<AGARI/>` can attribute
    /// the ron target correctly.
    pub last_revealed_tile_actor: Option<u8>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            seat: 0,
            hand: Vec::new(),
            melds: Vec::new(),
            in_riichi: false,
            live_wall: 70,
            last_kawa_tile: "?".to_string(),
            is_tsumo: false,
            is_3p: false,
            num_players: 4,
            last_revealed_tile_actor: None,
        }
    }
}

impl State {
    /// Convert relative seat (Tenhou's frame of reference) to absolute seat
    /// (mjai's frame of reference). Modulus tracks `num_players` so sanma
    /// remaps correctly.
    pub fn rel_to_abs(&self, rel: u8) -> u8 {
        (rel + self.seat) % self.num_players
    }

    /// Inverse of [`rel_to_abs`].
    pub fn abs_to_rel(&self, abs: u8) -> u8 {
        (abs + self.num_players - self.seat) % self.num_players
    }

    /// Reset per-kyoku fields. Called from `INIT`.
    pub fn reset_for_kyoku(&mut self) {
        self.hand.clear();
        self.melds.clear();
        self.in_riichi = false;
        self.live_wall = 70;
        self.last_kawa_tile = "?".to_string();
        self.is_tsumo = false;
        self.last_revealed_tile_actor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yonma_seat_round_trip() {
        let mut s = State::default();
        s.seat = 2;
        for abs in 0..4u8 {
            let rel = s.abs_to_rel(abs);
            assert_eq!(s.rel_to_abs(rel), abs);
        }
    }

    #[test]
    fn sanma_seat_round_trip() {
        let mut s = State::default();
        s.num_players = 3;
        s.is_3p = true;
        s.seat = 1;
        for abs in 0..3u8 {
            let rel = s.abs_to_rel(abs);
            assert_eq!(s.rel_to_abs(rel), abs);
        }
    }
}
