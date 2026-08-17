//! Which pen tool is active: the Hand/Select predicates the tablet handlers
//! branch on. Re-exported from `tablet` so call sites are unchanged.
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{ActiveOption, CanvasGrab};
use compositor_orchestration_seat_gesture_touch::touch::TouchMode;

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

/// Should the dial zoom right now? True in the Hand tool OR while a canvas pan is in
/// progress — there's no other sensible dial action mid-pan, so the dial always zooms
/// then (e.g. pan with the pen, then spin the dial to zoom).
pub fn dial_zoom_active(l: &Loop) -> bool {
    hand_active(l) || l.inner.canvas().position_updating
}
