//! Autoplay manager: subscribes to bot decisions + mjai events,
//! translates them into UI clicks dispatched via CDP.
//!
//! Lifecycle:
//! - Spawned by `crate::lib::run` when `cfg.autoplay.enabled = true`.
//! - One long-lived `tokio::select!` loop over `BotResponseBus` and
//!   `MjaiBus`. Bot responses drive clicks; mjai events update local
//!   per-game tracking state (`last_kawa_tile`, `last_self_tsumo`,
//!   `self_riichi_accepted`, `reach_state`). Before handling each bot
//!   response we flush queued mjai events so `Tsumo` updates land before
//!   discard planning (otherwise `last_self_tsumo` lags and clicks miss
//!   the intended tile).
//!
//! Failure modes are silent-by-design: if the page handle is missing
//! (chromium backend not running) or the canvas-rect query fails, the
//! manager logs a warning and skips the click. The bot pipeline is
//! untouched; the user can still play the round manually.
//!
//! Own discards: when `autoplay.majsoul.dahai_confirm_samples > 0`, the
//! planner is re-run that many times (with `dahai_confirm_gap_ms` between
//! samples); the click proceeds only if every sample agrees on the same
//! tile coordinates (`0` samples skips this check). If samples disagree,
//! the tracker has no click yet, or CDP click fails, we flush mjai and
//! **repeat** that confirm+click cycle until success (capped by
//! `DAHAI_CONFIRM_OUTER_MAX_ROUNDS`) instead of sitting out the turn.
//!
//! After executing our `dahai` clicks, we poll `GameTracker` until the
//! discard appears on our river (or time out). If it never shows up, we
//! assume the click missed (wrong window focus, etc.) and re-click with a
//! fresh canvas rect instead of waiting for the turn timer.

use crate::autoplay::cdp_input::{
    dispatch_click, dispatch_mouse_move, evaluate_canvas_rect, focus_page_for_input,
};
use crate::autoplay::context::{AutoplayContext, CanvasRect};
use crate::autoplay::majsoul::{tracked_discard_matches_bot_tile, MajsoulAutoplay};
use crate::autoplay::platform::{ActionContext, PlanResult, PlatformAutoplay, ReachState, Step};
use crate::bot::BotResponse;
use crate::config::{AppConfig, MajsoulAutoplayConfig};
use crate::event_bus::{BotResponseBus, MjaiBus};
use crate::game_state::snapshot::{GameStateSnapshot, Phase};
use crate::game_state::tracker::GameTracker;
use crate::schema::MjaiEvent;
use riichienv_core::action::Action;
use riichienv_core::state::legal_actions::GameStateLegalActions;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{debug, info, warn};

/// How long before a cached `CanvasRect` is treated as stale and re-queried.
const CANVAS_RECT_TTL: Duration = Duration::from_secs(2);

const TRACKER_SYNC_MS: u64 = 48;
const PLAN_RETRY_MAX: u32 = 3;
const PLAN_RETRY_GAP_MS: u64 = 55;
const CANVAS_RESOLVE_ATTEMPTS: u32 = 12;
const CANVAS_RESOLVE_GAP_MS: u64 = 150;
const CLICK_RETRY_MAX: u32 = 3;
const CLICK_RETRY_GAP_MS: u64 = 100;
/// Drain + gap so a same-turn `Tsumo` is applied before we read `last_self_tsumo`.
const MJAI_DRAIN_GAP_MS: u64 = 22;
/// After a failed discard-confirm attempt, pause before re-running the cycle.
const DAHAI_CONFIRM_OUTER_GAP_MS: u64 = 72;
/// Hard floor for own-discard confirmation. Wrong own discards are
/// irreversible, so runtime safety takes precedence over older configs that
/// used a lower value or disabled confirmation.
const DAHAI_CONFIRM_MIN_SAMPLES: u32 = 5;
/// Upper bound on outer discard-confirm rounds (each round = full multi-sample
/// agreement + click). Avoids infinite spin if CDP or the page is down.
const DAHAI_CONFIRM_OUTER_MAX_ROUNDS: u32 = 8;
/// After clicking our discard, poll the tracker briefly until this `dahai`
/// shows on our river. Keep this short: if the user moved the mouse or changed
/// focus and the click was dropped, waiting too long lets Majsoul's timer
/// auto-discard before we retry.
const DISCARD_VERIFY_TIMEOUT_MS: u64 = 2_000;
const DISCARD_VERIFY_POLL_MS: u64 = 80;
/// Without multi-sample confirm: extra full click+verify cycles after a
/// failed tracker check (e.g. wrong window focused).
const DISCARD_UNCONFIRMED_RECLICK_MAX: u32 = 5;
/// Neutral table point used to clear Majsoul's hover state after synthetic
/// clicks. Keep this away from the local hand, action buttons, and candidates.
const CURSOR_PARK_X_NORM: f64 = 8.0;
const CURSOR_PARK_Y_NORM: f64 = 4.5;

pub struct AutoplayManager {
    cfg: Arc<RwLock<AppConfig>>,
    ctx: Arc<AutoplayContext>,
    tracker: Arc<Mutex<GameTracker>>,
    mjai_bus: MjaiBus,
    platform: Arc<dyn PlatformAutoplay>,
    state: ManagerState,
}

#[derive(Default)]
struct ManagerState {
    in_kyoku: bool,
    last_kawa_tile: Option<String>,
    last_self_tsumo: Option<String>,
    self_riichi_accepted: bool,
    reach_state: ReachState,
    canvas_rect_at: Option<Instant>,
    mjai_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiscardVerifyResult {
    Expected,
    Different { actual: String },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscardPlanSignature {
    target_pai: String,
    tehai: Vec<String>,
    last_self_tsumo: Option<String>,
    phase: Phase,
    current_player: u8,
    river_len: usize,
}

#[derive(Debug, Clone)]
struct ConfirmedDiscardPlan {
    plan: PlanResult,
    coords: (f64, f64),
    signature: DiscardPlanSignature,
}

impl Default for ReachState {
    fn default() -> Self {
        Self::Idle
    }
}

impl AutoplayManager {
    pub fn new(
        cfg: Arc<RwLock<AppConfig>>,
        ctx: Arc<AutoplayContext>,
        tracker: Arc<Mutex<GameTracker>>,
        mjai_bus: MjaiBus,
    ) -> Self {
        Self {
            cfg,
            ctx,
            tracker,
            mjai_bus,
            // Only Majsoul is wired up today; future Tenhou impl swaps
            // here based on config.platform.kind at run start.
            platform: Arc::new(MajsoulAutoplay::new()),
            state: ManagerState::default(),
        }
    }

    /// Run forever. Returns `Err` only on bus closure (process exit).
    pub async fn run(mut self, response_bus: BotResponseBus) -> anyhow::Result<()> {
        let mut bot_rx = response_bus.subscribe();
        let mut mjai_rx = self.mjai_bus.subscribe();
        info!("autoplay manager started");

        loop {
            tokio::select! {
                msg = bot_rx.recv() => match msg {
                    Ok(resp) => {
                        // `BotResponse` and `MjaiBus` are different channels: we may receive
                        // the bot's Dahai before the matching `Tsumo`, leaving `last_self_tsumo`
                        // stale and shifting every click by one tile. Flush mjai first.
                        self.sync_mjai_before_bot_response(&mut mjai_rx).await;
                        if !self.response_matches_current_mjai(&resp) {
                            continue;
                        }
                        self.handle_bot_response(resp, &mut mjai_rx).await;
                    }
                    Err(RecvError::Lagged(n)) => warn!("autoplay: bot bus lagged {n}"),
                    Err(RecvError::Closed) => {
                        info!("autoplay: bot bus closed; exiting");
                        return Ok(());
                    }
                },
                msg = mjai_rx.recv() => match msg {
                    Ok(ev) => self.handle_mjai_event(&ev),
                    Err(RecvError::Lagged(n)) => warn!("autoplay: mjai bus lagged {n}"),
                    Err(RecvError::Closed) => {
                        info!("autoplay: mjai bus closed; exiting");
                        return Ok(());
                    }
                },
            }
        }
    }

    async fn handle_bot_response(
        &mut self,
        resp: BotResponse,
        mjai_rx: &mut broadcast::Receiver<MjaiEvent>,
    ) {
        // Re-read config every iteration so `cfg.autoplay.enabled` can be
        // toggled at runtime via the Settings UI without restarting.
        let cfg_guard = self.cfg.read().await;
        if !cfg_guard.autoplay.enabled {
            return;
        }
        let cfg = cfg_guard.autoplay.majsoul.clone();
        drop(cfg_guard);

        // Bot manager and game tracker both subscribe to the same MJAI bus;
        // scheduling order is undefined 鈥?see PLAN_RETRY_* below.
        tokio::time::sleep(Duration::from_millis(TRACKER_SYNC_MS)).await;
        self.drain_mjai_instant(mjai_rx);
        if !self.response_matches_current_mjai(&resp) {
            return;
        }

        if !self.click_scope_active(&resp).await {
            return;
        }

        let mut plan = PlanResult::default();
        let mut pack: Option<(u8, Vec<Action>, GameStateSnapshot, u8)> = None;

        for attempt in 0..PLAN_RETRY_MAX {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(PLAN_RETRY_GAP_MS)).await;
                self.drain_mjai_instant(mjai_rx);
                if !self.click_scope_active(&resp).await {
                    return;
                }
            }

            let Some((seat, legal, snap, np)) = self.read_tracker_for_plan(&resp).await else {
                return;
            };

            let action_ctx = ActionContext {
                action: &resp.action,
                snapshot: &snap,
                legal_actions: &legal,
                our_seat: seat,
                last_kawa_tile: self.state.last_kawa_tile.as_deref(),
                last_self_tsumo: self.state.last_self_tsumo.as_deref(),
                self_riichi_accepted: self.state.self_riichi_accepted,
                reach_state: self.state.reach_state,
                num_players: np,
                cfg: &cfg,
            };

            plan = self.platform.plan(&action_ctx);
            pack = Some((seat, legal, snap, np));

            if plan_should_retry_after_tracker_catchup(&resp, seat, &plan)
                && attempt + 1 < PLAN_RETRY_MAX
            {
                debug!(
                    "autoplay: plan had no click for own Dahai (attempt {}); retrying for tracker sync",
                    attempt + 1
                );
                continue;
            }
            break;
        }

        let Some((our_seat, _, _, _)) = pack else {
            return;
        };

        let initial_discard_confirm_n = effective_dahai_confirm_samples(&cfg);
        let is_own_dahai = matches!(
            &resp.action,
            MjaiEvent::Dahai { actor, .. } if *actor == our_seat
        );

        if is_own_dahai && initial_discard_confirm_n > 0 {
            for round in 0..DAHAI_CONFIRM_OUTER_MAX_ROUNDS {
                if !self.click_scope_active(&resp).await {
                    return;
                }

                let cfg_guard = self.cfg.read().await;
                if !cfg_guard.autoplay.enabled {
                    return;
                }
                let cfg_loop = cfg_guard.autoplay.majsoul.clone();
                drop(cfg_guard);

                let discard_confirm_n = effective_dahai_confirm_samples(&cfg_loop);

                match self
                    .triple_confirm_discard_plan(&resp, &cfg_loop, discard_confirm_n)
                    .await
                {
                    None => {
                        warn!(
                            "autoplay: discard confirm outer round {} 鈥?samples did not stabilize; re-syncing and retrying",
                            round + 1
                        );
                        self.sync_mjai_before_bot_response(mjai_rx).await;
                        if !self.click_scope_active(&resp).await {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(DAHAI_CONFIRM_OUTER_GAP_MS)).await;
                        continue;
                    }
                    Some(confirmed_plan) => {
                        plan = confirmed_plan.plan.clone();
                        info!(
                            "autoplay: discard target agreed {}x for {} at {:?} (outer round {})",
                            discard_confirm_n,
                            confirmed_plan.signature.target_pai,
                            confirmed_plan.coords,
                            round + 1
                        );

                        if plan.steps.is_empty() && !plan.inject_reach_for_followup {
                            warn!(
                                "autoplay: confirmed plan empty for {:?} 鈥?retrying discard cycle",
                                resp.action
                            );
                            self.sync_mjai_before_bot_response(mjai_rx).await;
                            if !self.click_scope_active(&resp).await {
                                return;
                            }
                            tokio::time::sleep(Duration::from_millis(DAHAI_CONFIRM_OUTER_GAP_MS))
                                .await;
                            continue;
                        }

                        let has_click = plan.steps.iter().any(|s| matches!(s, Step::Click { .. }));
                        if !has_click && !plan.inject_reach_for_followup {
                            warn!(
                                "autoplay: confirmed plan has no click for {:?} 鈥?retrying discard cycle",
                                resp.action
                            );
                            self.sync_mjai_before_bot_response(mjai_rx).await;
                            if !self.click_scope_active(&resp).await {
                                return;
                            }
                            tokio::time::sleep(Duration::from_millis(DAHAI_CONFIRM_OUTER_GAP_MS))
                                .await;
                            continue;
                        }

                        debug!(
                            "autoplay: action={:?} steps={} inject_reach={} await_riichi_dahai={}",
                            resp.action,
                            plan.steps.len(),
                            plan.inject_reach_for_followup,
                            plan.awaiting_riichi_dahai
                        );

                        info!(
                            "autoplay: executing {} step(s) for {:?}",
                            plan.steps.len(),
                            resp.action
                        );

                        if !self
                            .final_confirm_discard_plan(&resp, &cfg_loop, &confirmed_plan)
                            .await
                        {
                            warn!(
                                "autoplay: final pre-click discard confirmation failed for {:?}; retrying full confirm cycle",
                                resp.action
                            );
                            self.sync_mjai_before_bot_response(mjai_rx).await;
                            if !self.click_scope_active(&resp).await {
                                return;
                            }
                            tokio::time::sleep(Duration::from_millis(DAHAI_CONFIRM_OUTER_GAP_MS))
                                .await;
                            continue;
                        }

                        let river_tip = self.read_our_river_tip(our_seat).await;

                        if self
                            .execute_plan_clicks(&resp, &plan, &cfg_loop, round > 0)
                            .await
                        {
                            let confirmed = match &river_tip {
                                Some((len, last)) => {
                                    match self
                                        .own_discard_confirmed_after_clicks(
                                            mjai_rx,
                                            &resp,
                                            our_seat,
                                            *len,
                                            last.as_deref(),
                                        )
                                        .await
                                    {
                                        DiscardVerifyResult::Expected => true,
                                        DiscardVerifyResult::Different { actual } => {
                                            warn!(
                                                "autoplay: clicked discard advanced river with {actual}, not {:?}; stop retrying stale bot action",
                                                resp.action
                                            );
                                            true
                                        }
                                        DiscardVerifyResult::Missing => false,
                                    }
                                }
                                None => {
                                    debug!(
                                        "autoplay: skip discard verify 鈥?no tracker snapshot yet"
                                    );
                                    true
                                }
                            };
                            if confirmed {
                                self.apply_plan_side_effects(&resp, our_seat, &plan);
                                return;
                            }
                            warn!(
                                "autoplay: discard not confirmed on river 鈥?retrying full confirm cycle"
                            );
                            self.invalidate_canvas_cache().await;
                            self.sync_mjai_before_bot_response(mjai_rx).await;
                            if !self.click_scope_active(&resp).await {
                                return;
                            }
                            tokio::time::sleep(Duration::from_millis(DAHAI_CONFIRM_OUTER_GAP_MS))
                                .await;
                            continue;
                        }

                        warn!(
                            "autoplay: click sequence failed for discard 鈥?retrying full confirm cycle"
                        );
                        self.invalidate_canvas_cache().await;
                        self.sync_mjai_before_bot_response(mjai_rx).await;
                        if !self.click_scope_active(&resp).await {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(DAHAI_CONFIRM_OUTER_GAP_MS)).await;
                    }
                }
            }

            warn!(
                "autoplay: discard confirm exhausted {} outer rounds without success 鈥?giving up on this response",
                DAHAI_CONFIRM_OUTER_MAX_ROUNDS
            );
            return;
        }

        if plan.steps.is_empty() && !plan.inject_reach_for_followup {
            return;
        }

        let has_click = plan.steps.iter().any(|s| matches!(s, Step::Click { .. }));
        if !has_click && !plan.inject_reach_for_followup {
            warn!(
                "autoplay: plan has delays but no click for {:?} 鈥?tracker/bot desync or UI coords missing; check game view",
                resp.action
            );
            return;
        }

        debug!(
            "autoplay: action={:?} steps={} inject_reach={} await_riichi_dahai={}",
            resp.action,
            plan.steps.len(),
            plan.inject_reach_for_followup,
            plan.awaiting_riichi_dahai
        );

        info!(
            "autoplay: executing {} step(s) for {:?}",
            plan.steps.len(),
            resp.action
        );

        let linear_own_dahai = matches!(
            &resp.action,
            MjaiEvent::Dahai { actor, .. } if *actor == our_seat
        );
        let mut discard_reclicks = 0u32;

        loop {
            let river_tip = if linear_own_dahai {
                self.read_our_river_tip(our_seat).await
            } else {
                None
            };

            if !self
                .execute_plan_clicks(&resp, &plan, &cfg, discard_reclicks > 0)
                .await
            {
                warn!(
                    "autoplay: click sequence failed 鈥?aborting {:?}",
                    resp.action
                );
                return;
            }

            if !linear_own_dahai {
                break;
            }

            let confirmed = match &river_tip {
                Some((len, last)) => {
                    match self
                        .own_discard_confirmed_after_clicks(
                            mjai_rx,
                            &resp,
                            our_seat,
                            *len,
                            last.as_deref(),
                        )
                        .await
                    {
                        DiscardVerifyResult::Expected => true,
                        DiscardVerifyResult::Different { actual } => {
                            warn!(
                                "autoplay: clicked discard advanced river with {actual}, not {:?}; stop retrying stale bot action",
                                resp.action
                            );
                            true
                        }
                        DiscardVerifyResult::Missing => false,
                    }
                }
                None => {
                    debug!("autoplay: skip discard verify 鈥?no tracker snapshot");
                    true
                }
            };

            if confirmed {
                break;
            }

            discard_reclicks += 1;
            if discard_reclicks > DISCARD_UNCONFIRMED_RECLICK_MAX {
                warn!(
                    "autoplay: discard still not on river after {} re-click cycles 鈥?giving up {:?}",
                    DISCARD_UNCONFIRMED_RECLICK_MAX,
                    resp.action
                );
                return;
            }
            warn!(
                "autoplay: re-clicking discard (attempt {}) 鈥?window focus or canvas may be stale",
                discard_reclicks + 1
            );
            self.invalidate_canvas_cache().await;
            self.sync_mjai_before_bot_response(mjai_rx).await;
            if !self.click_scope_active(&resp).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(
                DISCARD_VERIFY_POLL_MS.saturating_mul(2),
            ))
            .await;
        }

        self.apply_plan_side_effects(&resp, our_seat, &plan);
    }

    fn tracker_discard_after_click(
        snap: &GameStateSnapshot,
        our_seat: u8,
        expected_pai: &str,
        river_len_before: usize,
        river_last_before: Option<&str>,
    ) -> DiscardVerifyResult {
        let Some(player) = snap.players.get(our_seat as usize) else {
            return DiscardVerifyResult::Missing;
        };
        let Some(last) = player.river.last() else {
            return DiscardVerifyResult::Missing;
        };
        let advanced = if player.river.len() > river_len_before {
            true
        } else {
            match river_last_before {
                Some(prev) => prev != last.tile.as_str(),
                None => false,
            }
        };
        if !advanced {
            return DiscardVerifyResult::Missing;
        }
        if tracked_discard_matches_bot_tile(&last.tile, expected_pai) {
            DiscardVerifyResult::Expected
        } else {
            DiscardVerifyResult::Different {
                actual: last.tile.clone(),
            }
        }
    }

    async fn read_our_river_tip(&self, our_seat: u8) -> Option<(usize, Option<String>)> {
        let t = self.tracker.lock().await;
        let snap = t.snapshot()?;
        let p = snap.players.get(our_seat as usize)?;
        Some((p.river.len(), p.river.last().map(|e| e.tile.clone())))
    }

    async fn wait_until_own_discard_tracked(
        &mut self,
        mjai_rx: &mut broadcast::Receiver<MjaiEvent>,
        our_seat: u8,
        expected_pai: &str,
        river_len_before: usize,
        river_last_before: Option<&str>,
    ) -> DiscardVerifyResult {
        let deadline = Instant::now() + Duration::from_millis(DISCARD_VERIFY_TIMEOUT_MS);
        while Instant::now() < deadline {
            self.drain_mjai_instant(mjai_rx);
            if !self.state.in_kyoku {
                warn!("autoplay: discard verify aborted because kyoku ended before confirmation");
                return DiscardVerifyResult::Missing;
            }
            let result = {
                let t = self.tracker.lock().await;
                t.snapshot()
                    .map(|snap| {
                        Self::tracker_discard_after_click(
                            &snap,
                            our_seat,
                            expected_pai,
                            river_len_before,
                            river_last_before,
                        )
                    })
                    .unwrap_or(DiscardVerifyResult::Missing)
            };
            match result {
                DiscardVerifyResult::Expected => {
                    info!(
                        "autoplay: tracker confirmed own discard {} (screen/focus check)",
                        expected_pai
                    );
                    return DiscardVerifyResult::Expected;
                }
                DiscardVerifyResult::Different { actual } => {
                    warn!(
                        "autoplay: tracker saw own river advance with {actual}, expected {expected_pai}; click target was wrong or UI shifted"
                    );
                    return DiscardVerifyResult::Different { actual };
                }
                DiscardVerifyResult::Missing => {}
            }
            tokio::time::sleep(Duration::from_millis(DISCARD_VERIFY_POLL_MS)).await;
        }
        warn!(
            "autoplay: discard {} not on river within {}ms 鈥?likely missed click",
            expected_pai, DISCARD_VERIFY_TIMEOUT_MS
        );
        DiscardVerifyResult::Missing
    }

    async fn own_discard_confirmed_after_clicks(
        &mut self,
        mjai_rx: &mut broadcast::Receiver<MjaiEvent>,
        resp: &BotResponse,
        our_seat: u8,
        river_len_before: usize,
        river_last_before: Option<&str>,
    ) -> DiscardVerifyResult {
        let MjaiEvent::Dahai { pai, actor, .. } = &resp.action else {
            return DiscardVerifyResult::Expected;
        };
        if *actor != our_seat {
            return DiscardVerifyResult::Expected;
        }
        self.wait_until_own_discard_tracked(
            mjai_rx,
            our_seat,
            pai,
            river_len_before,
            river_last_before,
        )
        .await
    }

    async fn execute_plan_clicks(
        &mut self,
        resp: &BotResponse,
        plan: &PlanResult,
        cfg: &MajsoulAutoplayConfig,
        skip_sleeps: bool,
    ) -> bool {
        for step in &plan.steps {
            match step {
                Step::Sleep { duration_ms } => {
                    if !skip_sleeps {
                        tokio::time::sleep(Duration::from_millis(*duration_ms as u64)).await;
                        if !self.click_scope_active(resp).await {
                            return false;
                        }
                    }
                }
                Step::Click { x_norm, y_norm } => {
                    if !self.click_scope_active(resp).await {
                        return false;
                    }
                    let rect = match self.canvas_rect_resolve_retrying().await {
                        Some(r) => r,
                        None => {
                            warn!("autoplay: no canvas rect immediately before click");
                            return false;
                        }
                    };
                    let (px, py) = rect.pixel(*x_norm, *y_norm);
                    if !rect.contains(px, py) {
                        warn!(
                            "autoplay: click ({px},{py}) outside canvas rect {:?}; aborting step sequence",
                            rect
                        );
                        return false;
                    }
                    let mut dispatched = false;
                    for c_attempt in 0..CLICK_RETRY_MAX {
                        if c_attempt > 0 {
                            tokio::time::sleep(Duration::from_millis(CLICK_RETRY_GAP_MS)).await;
                            if !self.click_scope_active(resp).await {
                                return false;
                            }
                        }
                        let page_guard = self.ctx.page.read().await;
                        let Some(page) = page_guard.as_ref() else {
                            drop(page_guard);
                            warn!(
                                "autoplay: no page handle before click (retry {}/{})",
                                c_attempt + 1,
                                CLICK_RETRY_MAX
                            );
                            continue;
                        };
                        if let Err(e) = focus_page_for_input(page).await {
                            drop(page_guard);
                            warn!(
                                "autoplay: focus page failed before click (retry {}/{}): {e:#}",
                                c_attempt + 1,
                                CLICK_RETRY_MAX
                            );
                            continue;
                        }
                        match dispatch_click(page, px, py, cfg.hover_delay_ms, cfg.click_hold_ms)
                            .await
                        {
                            Ok(()) => {
                                let (park_x, park_y) =
                                    rect.pixel(CURSOR_PARK_X_NORM, CURSOR_PARK_Y_NORM);
                                if let Err(e) = dispatch_mouse_move(page, park_x, park_y).await {
                                    warn!(
                                        "autoplay: cursor park move failed after click (retry {}/{}): {e:#}",
                                        c_attempt + 1,
                                        CLICK_RETRY_MAX
                                    );
                                }
                                drop(page_guard);
                                dispatched = true;
                                break;
                            }
                            Err(e) => {
                                drop(page_guard);
                                warn!(
                                    "autoplay: dispatch_click failed (retry {}/{}): {e:#}",
                                    c_attempt + 1,
                                    CLICK_RETRY_MAX
                                );
                            }
                        }
                    }
                    if !dispatched {
                        warn!("autoplay: giving up on click after per-click retries");
                        return false;
                    }
                }
            }
        }
        true
    }

    fn apply_plan_side_effects(&mut self, resp: &BotResponse, our_seat: u8, plan: &PlanResult) {
        // Path-B side effect: inject synthetic Reach so the bot will
        // emit the riichi-declaring dahai we need to click next.
        if plan.inject_reach_for_followup {
            self.state.reach_state = ReachState::AwaitingDahai;
            let synthetic = MjaiEvent::reach_prompt_riichi_dahai(our_seat);
            if let Err(e) = self.mjai_bus.send(synthetic) {
                warn!(
                    "autoplay: synthetic Reach (Path B) not delivered ({e:?}) 鈥?bot may not emit riichi dahai; \
                     manual click or new round may be needed"
                );
            } else {
                info!("autoplay: injected synthetic Reach (Path B) for seat {our_seat}");
            }
        }

        // After a successful Path-B follow-up dahai, return to Idle.
        // The check is conservative: only flip on Dahai by our seat.
        if matches!(self.state.reach_state, ReachState::AwaitingDahai) {
            if let MjaiEvent::Dahai { actor, .. } = &resp.action {
                if *actor == our_seat {
                    self.state.reach_state = ReachState::Idle;
                }
            }
        }
    }

    fn handle_mjai_event(&mut self, ev: &MjaiEvent) {
        let mjai_seq = self.state.mjai_seq.saturating_add(1);
        match ev {
            MjaiEvent::StartGame { .. } | MjaiEvent::EndGame => {
                let canvas_at = self.state.canvas_rect_at;
                self.state = ManagerState::default();
                self.state.canvas_rect_at = canvas_at;
                self.state.mjai_seq = mjai_seq;
            }
            MjaiEvent::StartKyoku { .. } => {
                // Per-kyoku reset: keep last seen rect cache, drop
                // everything else.
                let canvas_at = self.state.canvas_rect_at;
                self.state = ManagerState::default();
                self.state.canvas_rect_at = canvas_at;
                self.state.mjai_seq = mjai_seq;
                self.state.in_kyoku = true;
            }
            MjaiEvent::EndKyoku => {
                let canvas_at = self.state.canvas_rect_at;
                self.state = ManagerState::default();
                self.state.canvas_rect_at = canvas_at;
                self.state.mjai_seq = mjai_seq;
            }
            MjaiEvent::Tsumo { actor, pai } => {
                if let Some(seat) = self.our_seat_cached() {
                    if *actor == seat {
                        self.state.last_self_tsumo = Some(pai.clone());
                    }
                }
            }
            MjaiEvent::Dahai { actor, pai, .. } => {
                self.state.last_kawa_tile = Some(pai.clone());
                if let Some(seat) = self.our_seat_cached() {
                    if *actor == seat {
                        self.state.last_self_tsumo = None;
                    }
                }
            }
            MjaiEvent::ReachAccepted { actor } => {
                if let Some(seat) = self.our_seat_cached() {
                    if *actor == seat {
                        self.state.self_riichi_accepted = true;
                    }
                }
            }
            MjaiEvent::Chi { actor, .. }
            | MjaiEvent::Pon { actor, .. }
            | MjaiEvent::Daiminkan { actor, .. }
            | MjaiEvent::Ankan { actor, .. }
            | MjaiEvent::Kakan { actor, .. } => {
                if let Some(seat) = self.our_seat_cached() {
                    if *actor == seat {
                        self.state.last_self_tsumo = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn response_matches_current_mjai(&self, resp: &BotResponse) -> bool {
        let Some(trigger_seq) = resp.trigger_seq else {
            return true;
        };
        if trigger_seq >= self.state.mjai_seq {
            return true;
        }
        if !matches!(resp.action, MjaiEvent::None) {
            warn!(
                "autoplay: dropping stale bot response {:?}; trigger_seq={} current_mjai_seq={}",
                resp.action, trigger_seq, self.state.mjai_seq
            );
        }
        false
    }

    /// Best-effort seat lookup that avoids blocking on the tracker mutex
    /// inside the mjai event handler (which runs synchronously in the
    /// select arm). Falls back to `None` if the lock is contended;
    /// missing the seat once is harmless, the next event will catch up.
    fn our_seat_cached(&self) -> Option<u8> {
        self.tracker.try_lock().ok().and_then(|t| t.our_seat())
    }

    async fn click_scope_active(&self, resp: &BotResponse) -> bool {
        if !self.state.in_kyoku {
            debug!(
                "autoplay: skipping {:?} because no active kyoku is tracked",
                resp.action
            );
            return false;
        }
        let tracker = self.tracker.lock().await;
        let Some(our_seat) = tracker.our_seat() else {
            debug!(
                "autoplay: skipping {:?} because tracker has no active seat",
                resp.action
            );
            return false;
        };
        let Some(snapshot) = tracker.snapshot() else {
            debug!(
                "autoplay: skipping {:?} because tracker has no live snapshot",
                resp.action
            );
            return false;
        };
        if snapshot.is_done || snapshot.our_seat != Some(our_seat) {
            debug!(
                "autoplay: skipping {:?} because snapshot is not live (done={} snapshot_seat={:?} tracker_seat={})",
                resp.action,
                snapshot.is_done,
                snapshot.our_seat,
                our_seat
            );
            return false;
        }
        if !matches!(resp.action, MjaiEvent::None | MjaiEvent::Ryukyoku { .. })
            && !snapshot_is_live_for_action(&resp.action, &snapshot, our_seat)
        {
            debug!(
                "autoplay: skipping {:?} because live snapshot/action gate rejected phase={:?} current={} our={}",
                resp.action,
                snapshot.phase,
                snapshot.current_player,
                our_seat
            );
            return false;
        }
        true
    }

    async fn sync_mjai_before_bot_response(&mut self, rx: &mut broadcast::Receiver<MjaiEvent>) {
        self.drain_mjai_instant(rx);
        tokio::time::sleep(Duration::from_millis(MJAI_DRAIN_GAP_MS)).await;
        self.drain_mjai_instant(rx);
    }

    fn drain_mjai_instant(&mut self, rx: &mut broadcast::Receiver<MjaiEvent>) {
        loop {
            match rx.try_recv() {
                Ok(ev) => self.handle_mjai_event(&ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Closed) => break,
                Err(TryRecvError::Lagged(n)) => {
                    warn!(
                        "autoplay: mjai lagged {n} events while flushing before bot 鈥?discard coords may be wrong for a turn"
                    );
                    break;
                }
            }
        }
    }

    /// Re-plan the same bot `Dahai` `n` times; only return the last plan if
    /// every sample's target tile, hand signature, and final click match.
    async fn triple_confirm_discard_plan(
        &self,
        resp: &BotResponse,
        cfg: &MajsoulAutoplayConfig,
        samples: u32,
    ) -> Option<ConfirmedDiscardPlan> {
        let gap_ms = (cfg.dahai_confirm_gap_ms as u64).min(2000);
        let mut coords_acc: Option<(f64, f64)> = None;
        let mut sig_acc: Option<DiscardPlanSignature> = None;
        let mut last_plan = PlanResult::default();
        for i in 0..samples {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(gap_ms)).await;
            }
            let (seat, legal, snap, np) = self.read_tracker_for_plan(resp).await?;
            let signature =
                discard_plan_signature(resp, &snap, seat, self.state.last_self_tsumo.as_deref())?;
            let action_ctx = ActionContext {
                action: &resp.action,
                snapshot: &snap,
                legal_actions: &legal,
                our_seat: seat,
                last_kawa_tile: self.state.last_kawa_tile.as_deref(),
                last_self_tsumo: self.state.last_self_tsumo.as_deref(),
                self_riichi_accepted: self.state.self_riichi_accepted,
                reach_state: self.state.reach_state,
                num_players: np,
                cfg,
            };
            last_plan = self.platform.plan(&action_ctx);
            let c = last_click_coords(&last_plan)?;
            match &sig_acc {
                None => sig_acc = Some(signature),
                Some(prev) if prev != &signature => {
                    warn!(
                        "autoplay: discard sample {}/{} target/hand changed {:?} vs {:?}",
                        i + 1,
                        samples,
                        prev,
                        signature
                    );
                    return None;
                }
                Some(_) => {}
            }
            match coords_acc {
                None => coords_acc = Some(c),
                Some(prev) if !coords_close(prev, c) => {
                    warn!(
                        "autoplay: discard sample {}/{} mismatch {:?} vs {:?}",
                        i + 1,
                        samples,
                        prev,
                        c
                    );
                    return None;
                }
                Some(_) => {}
            }
        }
        Some(ConfirmedDiscardPlan {
            plan: last_plan,
            coords: coords_acc?,
            signature: sig_acc?,
        })
    }

    async fn final_confirm_discard_plan(
        &self,
        resp: &BotResponse,
        cfg: &MajsoulAutoplayConfig,
        expected: &ConfirmedDiscardPlan,
    ) -> bool {
        let Some((seat, legal, snap, np)) = self.read_tracker_for_plan(resp).await else {
            return false;
        };
        let Some(signature) =
            discard_plan_signature(resp, &snap, seat, self.state.last_self_tsumo.as_deref())
        else {
            return false;
        };
        if signature != expected.signature {
            warn!(
                "autoplay: final discard target/hand changed {:?} vs {:?}",
                expected.signature, signature
            );
            return false;
        }
        let action_ctx = ActionContext {
            action: &resp.action,
            snapshot: &snap,
            legal_actions: &legal,
            our_seat: seat,
            last_kawa_tile: self.state.last_kawa_tile.as_deref(),
            last_self_tsumo: self.state.last_self_tsumo.as_deref(),
            self_riichi_accepted: self.state.self_riichi_accepted,
            reach_state: self.state.reach_state,
            num_players: np,
            cfg,
        };
        let plan = self.platform.plan(&action_ctx);
        let Some(coords) = last_click_coords(&plan) else {
            return false;
        };
        if !coords_close(coords, expected.coords) {
            warn!(
                "autoplay: final discard coords changed {:?} vs {:?}",
                expected.coords, coords
            );
            return false;
        }
        true
    }

    async fn read_tracker_for_plan(
        &self,
        resp: &BotResponse,
    ) -> Option<(u8, Vec<Action>, GameStateSnapshot, u8)> {
        let tracker = self.tracker.lock().await;
        let our_seat = match tracker.our_seat() {
            Some(s) => s,
            None => {
                if !matches!(resp.action, MjaiEvent::None) {
                    warn!(
                        "autoplay: no seat (start_game.id missing?) 鈥?skipping {:?}",
                        resp.action
                    );
                }
                return None;
            }
        };
        let snapshot = match tracker.snapshot() {
            Some(s) => s,
            None => {
                if !matches!(resp.action, MjaiEvent::None) {
                    warn!(
                        "autoplay: tracker has no snapshot yet 鈥?skipping {:?}",
                        resp.action
                    );
                }
                return None;
            }
        };
        if !snapshot_is_live_for_action(&resp.action, &snapshot, our_seat) {
            debug!(
                "autoplay: snapshot/action gate rejected {:?} phase={:?} current={} our={} done={}",
                resp.action, snapshot.phase, snapshot.current_player, our_seat, snapshot.is_done
            );
            return None;
        }
        let num_players = snapshot.num_players;
        let legal_actions: Vec<Action> = if num_players == 3 {
            tracker
                .state_3p()
                .map(|s| {
                    let _ = s;
                    Vec::new()
                })
                .unwrap_or_default()
        } else {
            tracker
                .state()
                .map(|s| s._get_legal_actions_internal(our_seat))
                .unwrap_or_default()
        };
        Some((our_seat, legal_actions, snapshot, num_players))
    }

    async fn invalidate_canvas_cache(&mut self) {
        self.state.canvas_rect_at = None;
        *self.ctx.canvas_rect.write().await = None;
    }

    async fn canvas_rect_resolve_retrying(&mut self) -> Option<CanvasRect> {
        for attempt in 0..CANVAS_RESOLVE_ATTEMPTS {
            if attempt > 0 {
                self.invalidate_canvas_cache().await;
                tokio::time::sleep(Duration::from_millis(CANVAS_RESOLVE_GAP_MS)).await;
            }
            let log_failures = attempt + 1 == CANVAS_RESOLVE_ATTEMPTS;
            if let Some(r) = self.canvas_rect_resolve(log_failures).await {
                return Some(r);
            }
        }
        None
    }

    async fn canvas_rect_resolve(&mut self, log_failures: bool) -> Option<CanvasRect> {
        let now = Instant::now();
        if let Some(at) = self.state.canvas_rect_at {
            if now.duration_since(at) < CANVAS_RECT_TTL {
                if let Some(r) = *self.ctx.canvas_rect.read().await {
                    return Some(r);
                }
            }
        }
        // Re-query.
        let page_guard = self.ctx.page.read().await;
        let Some(p) = page_guard.as_ref() else {
            if log_failures {
                warn!(
                    "autoplay: no Chromium page handle 鈥?open Majsoul in the Akagi browser until logs show \
                     \"autoplay: page handle bound\" (game WebSocket must connect)"
                );
            } else {
                debug!("autoplay: canvas resolve skipped 鈥?no page handle (will retry)");
            }
            return None;
        };
        let page = p.clone();
        drop(page_guard);
        match evaluate_canvas_rect(&page).await {
            Ok(rect) => {
                *self.ctx.canvas_rect.write().await = Some(rect);
                self.state.canvas_rect_at = Some(now);
                Some(rect)
            }
            Err(e) => {
                if log_failures {
                    warn!("autoplay: could not read game canvas rect: {e:#}");
                } else {
                    debug!("autoplay: canvas rect query failed (will retry): {e:#}");
                }
                None
            }
        }
    }
}

fn last_click_coords(plan: &PlanResult) -> Option<(f64, f64)> {
    plan.steps.iter().rev().find_map(|s| {
        if let Step::Click { x_norm, y_norm } = s {
            Some((*x_norm, *y_norm))
        } else {
            None
        }
    })
}

fn effective_dahai_confirm_samples(cfg: &MajsoulAutoplayConfig) -> u32 {
    cfg.dahai_confirm_samples
        .clamp(DAHAI_CONFIRM_MIN_SAMPLES, 20)
}

fn discard_plan_signature(
    resp: &BotResponse,
    snap: &GameStateSnapshot,
    our_seat: u8,
    last_self_tsumo: Option<&str>,
) -> Option<DiscardPlanSignature> {
    let MjaiEvent::Dahai { actor, pai, .. } = &resp.action else {
        return None;
    };
    if *actor != our_seat {
        return None;
    }
    let player = snap.players.get(our_seat as usize)?;
    if !hand_snapshot_contains_target(&player.tehai, pai, last_self_tsumo) {
        warn!(
            "autoplay: discard target {pai} is not present in current hand {:?} / tsumohai {:?}",
            player.tehai, last_self_tsumo
        );
        return None;
    }
    let mut tehai = player.tehai.clone();
    tehai.sort_by(|a, b| compare_pai_for_signature(a, b));
    Some(DiscardPlanSignature {
        target_pai: pai.clone(),
        tehai,
        last_self_tsumo: last_self_tsumo.map(str::to_owned),
        phase: snap.phase.clone(),
        current_player: snap.current_player,
        river_len: player.river.len(),
    })
}

fn hand_snapshot_contains_target(
    tehai: &[String],
    target: &str,
    last_self_tsumo: Option<&str>,
) -> bool {
    tehai.iter().any(|t| t == target) || last_self_tsumo == Some(target)
}

fn compare_pai_for_signature(a: &String, b: &String) -> std::cmp::Ordering {
    crate::bridge::majsoul::tile::compare_pai(a, b)
}

fn coords_close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
}

fn snapshot_is_live_for_action(action: &MjaiEvent, snap: &GameStateSnapshot, our_seat: u8) -> bool {
    if snap.is_done || snap.our_seat != Some(our_seat) {
        return false;
    }

    match action {
        MjaiEvent::Dahai { actor, .. }
        | MjaiEvent::Reach { actor, .. }
        | MjaiEvent::Ankan { actor, .. }
        | MjaiEvent::Kakan { actor, .. }
        | MjaiEvent::Kita { actor, .. } => {
            *actor == our_seat && snap.phase == Phase::WaitAct && snap.current_player == our_seat
        }
        MjaiEvent::Chi { actor, .. }
        | MjaiEvent::Pon { actor, .. }
        | MjaiEvent::Daiminkan { actor, .. } => {
            *actor == our_seat && snap.phase == Phase::WaitResponse
        }
        MjaiEvent::Hora { actor, target, .. } => {
            if *actor != our_seat {
                return false;
            }
            *target < snap.num_players && snap.phase == Phase::WaitResponse
        }
        MjaiEvent::Ryukyoku { .. } | MjaiEvent::None => true,
        _ => false,
    }
}

fn plan_should_retry_after_tracker_catchup(
    resp: &BotResponse,
    our_seat: u8,
    plan: &PlanResult,
) -> bool {
    let MjaiEvent::Dahai { actor, .. } = &resp.action else {
        return false;
    };
    if *actor != our_seat {
        return false;
    }
    let has_click = plan.steps.iter().any(|s| matches!(s, Step::Click { .. }));
    !has_click && !plan.inject_reach_for_followup
}

/// Spawn point for the autoplay loop. Wired by `crate::lib::run` so the
/// `tauri::async_runtime` Tokio runtime is the host.
pub async fn run_autoplay_manager(
    cfg: Arc<RwLock<AppConfig>>,
    ctx: Arc<AutoplayContext>,
    tracker: Arc<Mutex<GameTracker>>,
    mjai_bus: MjaiBus,
    response_bus: BotResponseBus,
) -> anyhow::Result<()> {
    AutoplayManager::new(cfg, ctx, tracker, mjai_bus)
        .run(response_bus)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::mjai_bus;

    fn manager_for_seq_tests() -> AutoplayManager {
        AutoplayManager::new(
            Arc::new(RwLock::new(AppConfig::default())),
            Arc::new(AutoplayContext::new()),
            crate::game_state::tracker::new_handle(),
            mjai_bus(),
        )
    }

    fn dahai_response(trigger_seq: Option<u64>) -> BotResponse {
        BotResponse {
            action: MjaiEvent::Dahai {
                actor: 0,
                pai: "5m".into(),
                tsumogiri: false,
            },
            meta: None,
            trigger_seq,
        }
    }

    #[test]
    fn response_sequence_accepts_current_decision() {
        let mut mgr = manager_for_seq_tests();
        mgr.state.mjai_seq = 7;
        assert!(mgr.response_matches_current_mjai(&dahai_response(Some(7))));
    }

    #[test]
    fn response_sequence_allows_response_that_arrives_before_mjai_catchup() {
        let mut mgr = manager_for_seq_tests();
        mgr.state.mjai_seq = 6;
        assert!(mgr.response_matches_current_mjai(&dahai_response(Some(7))));
    }

    #[test]
    fn response_sequence_rejects_stale_decision() {
        let mut mgr = manager_for_seq_tests();
        mgr.state.mjai_seq = 8;
        assert!(!mgr.response_matches_current_mjai(&dahai_response(Some(7))));
    }

    #[test]
    fn response_sequence_allows_legacy_unstamped_response() {
        let mgr = manager_for_seq_tests();
        assert!(mgr.response_matches_current_mjai(&dahai_response(None)));
    }

    #[test]
    fn mjai_sequence_survives_round_resets() {
        let mut mgr = manager_for_seq_tests();
        mgr.handle_mjai_event(&MjaiEvent::StartGame {
            names: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            kyoku_first: None,
            aka_flag: None,
            id: Some(0),
            num_players: 4,
        });
        assert_eq!(mgr.state.mjai_seq, 1);
        mgr.handle_mjai_event(&MjaiEvent::StartKyoku {
            bakaze: "E".into(),
            dora_marker: "1m".into(),
            kyoku: 1,
            honba: 0,
            kyotaku: 0,
            oya: 0,
            scores: vec![25000; 4],
            tehais: vec![vec!["1m".into(); 13]; 4],
            num_players: 4,
        });
        assert_eq!(mgr.state.mjai_seq, 2);
        mgr.handle_mjai_event(&MjaiEvent::EndKyoku);
        assert_eq!(mgr.state.mjai_seq, 3);
    }
}
