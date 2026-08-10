//! Guide popups from the input rim: open on an empty-canvas right-click or
//! double-click, dismiss on anything else. Only the DESIRE is written here
//! (`GuideState`); the reconcilers build and tear down the surfaces on the
//! render path, which is what lets dismissal live in the raw input handlers.

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::Status;
use compositor_y5_guide_state_base::state::{DOUBLE_CLICK_MS, DOUBLE_CLICK_SLOP, GUIDE, GUIDE_MUT};
use compositor_y5_surface_interface_base::hit::surface_under_filtered;
use compositor_y5_window_interface_draw::visible::DrawWindow;

/// `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

pub fn showing(state: &Loop) -> bool {
    state.inner.kernel.get(&GUIDE).showing()
}

/// Dismiss both popups. Clears the DESIRE only (the reconciler destroys the
/// surfaces next frame), so this is safe to call from anywhere.
pub fn close(state: &mut Loop) {
    let guide = state.inner.kernel.get_mut(&GUIDE_MUT);
    guide.menu_at = None;
    guide.help_open = false;
}

/// Swap the menu for the help panel (the Help entry was clicked).
pub fn open_help(state: &mut Loop) {
    let guide = state.inner.kernel.get_mut(&GUIDE_MUT);
    guide.menu_at = None;
    guide.help_open = true;
}

/// Open the inline shader editor (the Shader entry was clicked).
pub fn open_shader(state: &mut Loop) {
    state.inner.kernel.get_mut(&GUIDE_MUT).shader_open = true;
}

/// Any key press dismisses an open popup. The key is NOT swallowed — the popup
/// is an overlay on the canvas, not a modal.
///
/// The shader editor is the exception, and only Escape closes it. It is meant to
/// be left up while the desktop it edits is used, so dismissing it on the next
/// keystroke would take it away in the middle of the job it exists for — whereas
/// the menu and the help panel are things you look at once and move on from.
pub fn on_key(state: &mut Loop, sym: u32) {
    if sym == smithay::input::keyboard::keysyms::KEY_Escape {
        state.inner.kernel.get_mut(&GUIDE_MUT).shader_open = false;
    }
    if showing(state) {
        close(state);
    }
}

/// A pointer press. Returns true when the press was consumed (it summoned the
/// menu) and must not go on to pan the canvas or reach a client.
///
/// Order matters: a press outside dismisses FIRST, so the same press can then
/// summon a fresh menu at its own position rather than leaving the old one.
pub fn on_press(state: &mut Loop, button: u32, time: u32) -> bool {
    let over = over_popup(state);
    if showing(state) && !over {
        close(state);
    }
    if over || !summonable(state) {
        return false;
    }
    let Some(cursor) = empty_canvas_at(state) else {
        state.inner.kernel.get_mut(&GUIDE_MUT).last_click = None;
        return false;
    };
    let repeat = matches!(state.inner.kernel.get(&GUIDE).last_click, Some((t, x, y))
        if time.saturating_sub(t) <= DOUBLE_CLICK_MS && (cursor.0 - x).hypot(cursor.1 - y) <= DOUBLE_CLICK_SLOP);
    if button == BTN_RIGHT || (button == BTN_LEFT && repeat) {
        let guide = state.inner.kernel.get_mut(&GUIDE_MUT);
        guide.menu_at = Some(cursor);
        guide.help_open = false;
        guide.last_click = None;
        return true;
    }
    if button == BTN_LEFT {
        state.inner.kernel.get_mut(&GUIDE_MUT).last_click = Some((time, cursor.0, cursor.1));
    }
    false
}

/// The world-logical cursor, if the pointer is over bare canvas — no window, no
/// layer, no iced surface of any kind.
fn empty_canvas_at(state: &Loop) -> Option<(f64, f64)> {
    let at = state.state.seat.seat.get_pointer()?.current_location();
    match surface_under_filtered(state, at, &|hit| hit.window().is_none_or(|w| w.visible(state))) {
        Some(_) => None,
        None => Some((at.x, at.y)),
    }
}

/// True when the pointer is over one of our own popups — that press belongs to
/// the popup (a button click) and must neither dismiss nor re-summon it.
///
/// The shader editor counts. It is not dismissible by an outside click, but a
/// press ON it must still not be read as a press on bare canvas: without this,
/// dragging one of its sliders would summon the menu underneath.
fn over_popup(state: &Loop) -> bool {
    let guide = state.inner.kernel.get(&GUIDE);
    let Some(at) = state.state.seat.seat.get_pointer().map(|p| p.current_location()) else { return false };
    surface_under_filtered(state, at, &|_| true)
        .and_then(|hit| hit.iced_handle())
        .is_some_and(|id| {
            guide.menu == Some(id) || guide.help == Some(id) || guide.shader == Some(id)
        })
}

/// The canvas has to actually be the thing on screen.
fn summonable(state: &Loop) -> bool {
    !state.inner.overview().visible && !matches!(state.inner.status, Status::Locked { .. })
}
