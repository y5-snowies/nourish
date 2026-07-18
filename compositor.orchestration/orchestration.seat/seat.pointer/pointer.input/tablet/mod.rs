//! Native graphics-tablet (pen / stylus) input, sibling of `touch/`.
//!
//! The four `InputEvent::TabletTool*` variants are routed here from
//! `delegate_main`. The whole `zwp_tablet_manager_v2` protocol is implemented
//! externally on `Dispatch.tablet` (see `wire.tablet`); these handlers resolve the
//! pen position into y5-world space (reusing the pointer's absolute→world path),
//! hit-test with the SAME `surface_under_filtered` pointer/touch use, and either
//! forward native tool events to a tablet-aware client or (on the canvas) pan.
//!
//! The pen drives `wl_pointer` (the cursor follows it) for everything that isn't a
//! native tool stroke. The stroke role (`Tablet` draw vs `Pointer` mouse/pan) is
//! latched at tip-down and held until tip-up / proximity-out (`wire.tablet::Stroke`),
//! like `touch/session.rs` latches its `Role`. In the canvas **Hand grab** the pen
//! is a navigation tool — tip-drag pans the world and the dial zooms (see
//! [`hand_active`]) — so it never draws or forwards hover while the grab is held.

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{ActiveOption, CanvasGrab};
use compositor_orchestration_seat_gesture_touch::touch::TouchMode;

pub mod action;
pub mod axis;
pub mod button;
pub mod client;
pub mod coords;
pub mod cursor;
pub mod inject;
pub mod pad;
pub mod proximity;
pub mod tip;

/// Is the Hand (navigation) tool active for the pen? True for BOTH the mouse/keyboard
/// canvas Hand grab (`CanvasGrab::Active(Hand)`) AND the touch pane's Hand mode
/// (`TouchMode::Hand`), so picking Hand in the touch menu also drives the pen. In hand
/// mode the pen navigates (tip-drag pans anywhere, dial zooms) instead of drawing.
pub fn hand_active(l: &Loop) -> bool {
    matches!(l.inner.canvas().Grab, CanvasGrab::Active(ActiveOption::Hand))
        || l.inner.touch.tool_mode == TouchMode::Hand
}

/// Is the Select tool active (touch pane's Select mode)? The pen then arms the same
/// transient select grab the touch session uses, so a pen tip-drag rubber-band selects.
pub fn select_active(l: &Loop) -> bool {
    l.inner.touch.tool_mode == TouchMode::Select
}
