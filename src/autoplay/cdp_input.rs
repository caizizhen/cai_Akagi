//! Thin wrappers around chromiumoxide for the autoplay manager.
//!
//! Centralised so the click + canvas-rect query logic can be unit-mocked
//! and the manager keeps a single dependency on chromiumoxide types.

use crate::autoplay::context::CanvasRect;
use anyhow::{anyhow, Context, Result};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
};
use chromiumoxide::layout::Point;
use chromiumoxide::page::Page;
use std::time::Duration;

/// Bring the game tab to the foreground and nudge DOM focus back to the page
/// before dispatching synthetic input. This is best-effort: CDP input usually
/// works without OS focus, but Mahjong Soul can drop clicks when the tab/page
/// lost focus after the user interacted elsewhere.
pub async fn focus_page_for_input(page: &Page) -> Result<()> {
    page.activate().await.context("CDP activate target")?;
    page.bring_to_front()
        .await
        .context("CDP bring page to front")?;
    let _ = page
        .evaluate("(()=>{window.focus();document.body&&document.body.focus&&document.body.focus();return true;})()")
        .await
        .context("CDP focus DOM")?;
    Ok(())
}

/// Dispatch a single mouse click at `(x, y)` (CSS pixels) as four CDP
/// events, with mandatory hover before press:
///
/// 1. `mouseMoved` to `(x, y)`
/// 2. sleep `hover_delay_ms` (≥100ms — Laya's input system samples hover
///    state before mousedown registers a hit on a tile sprite)
/// 3. `mousePressed`
/// 4. sleep `click_hold_ms`
/// 5. `mouseReleased`
///
/// `chromiumoxide::Page::click` collapses 3+5 into back-to-back frames
/// without the hover delay, which Majsoul drops on the floor for hand
/// tiles. Hand-rolling the sequence is required.
pub async fn dispatch_click(
    page: &Page,
    x: f64,
    y: f64,
    hover_delay_ms: u32,
    click_hold_ms: u32,
) -> Result<()> {
    let pt = Point::new(x, y);
    page.move_mouse(pt).await.context("CDP move_mouse")?;
    if hover_delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(hover_delay_ms as u64)).await;
    }

    let press = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MousePressed)
        .x(pt.x)
        .y(pt.y)
        .button(MouseButton::Left)
        .click_count(1)
        .build()
        .map_err(|e| anyhow!("build mousePressed: {e}"))?;
    page.execute(press).await.context("CDP mousePressed")?;

    if click_hold_ms > 0 {
        tokio::time::sleep(Duration::from_millis(click_hold_ms as u64)).await;
    }

    let release = DispatchMouseEventParams::builder()
        .r#type(DispatchMouseEventType::MouseReleased)
        .x(pt.x)
        .y(pt.y)
        .button(MouseButton::Left)
        .click_count(1)
        .build()
        .map_err(|e| anyhow!("build mouseReleased: {e}"))?;
    page.execute(release).await.context("CDP mouseReleased")?;

    Ok(())
}

/// Move Chromium's page-local mouse position without pressing a button.
///
/// This is separate from [`dispatch_click`] so callers can park the cursor in a
/// neutral area after a click. Majsoul keeps tile/button hover state from the
/// last mouse position, and leaving that state on a hand tile can make later
/// clicks depend on where the previous click ended.
pub async fn dispatch_mouse_move(page: &Page, x: f64, y: f64) -> Result<()> {
    let pt = Point::new(x, y);
    page.move_mouse(pt).await.context("CDP move_mouse")?;
    Ok(())
}

/// Read the game canvas's `getBoundingClientRect()` via `Runtime.evaluate`.
///
/// Majsoul may attach more than one `<canvas>` (e.g. overlays). Pick the
/// largest by layout area so we target the main Laya game surface.
pub async fn evaluate_canvas_rect(page: &Page) -> Result<CanvasRect> {
    // IIFE so `Runtime.evaluate` returns a single value, not a Promise.
    // `is_likely_js_function` in chromiumoxide picks the right CDP call
    // based on whether the expression looks like a function — we wrap
    // in `(()=>{...})()` to ensure plain-expression evaluation.
    let expr = "(()=>{const list=document.getElementsByTagName('canvas');\
                if(!list.length)return null;\
                let best=null,bestA=0;\
                for(const c of list){\
                  const r=c.getBoundingClientRect();\
                  const a=r.width*r.height;\
                  if(a>bestA){bestA=a;best={x:r.x,y:r.y,width:r.width,height:r.height};}\
                }\
                return best;})()";
    let result = page
        .evaluate(expr)
        .await
        .context("CDP evaluate canvas rect")?;
    let value = result
        .value()
        .ok_or_else(|| anyhow!("canvas rect: no value returned"))?;
    if value.is_null() {
        return Err(anyhow!("canvas rect: page has no <canvas> element"));
    }
    let rect: CanvasRect = serde_json::from_value(value.clone())
        .context("canvas rect: deserialise from page value")?;
    Ok(rect)
}
