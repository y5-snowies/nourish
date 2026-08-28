//! Executes a resolved [`PenAction`] for a pad/stylus button **edge**. Continuous
//! controls (the dial) resolve their own action at the source; here we act only on a
//! discrete press/release. `Passthrough` is handled by the caller (it forwards the
//! native protocol event); the dial-only `Zoom`/`Wheel` no-op in a button context.

use smithay::input::pointer::CursorIcon;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{ActiveOption, CanvasGrab};
use compositor_model_environment_preference_base::base::PenAction;
use crate::tablet::inject;

/// Toggle the canvas Hand grab (navigation mode) — the same state + forced grab
/// cursor the keyboard hand-tool chord drives, so pen, mouse and keyboard share one
/// mode (`ActiveOption::Hand`).
pub fn toggle_hand(_loop: &mut Loop) {
    if matches!(_loop.inner.canvas().Grab, CanvasGrab::Active(ActiveOption::Hand)) {
        _loop.inner.canvas_mut().Grab = CanvasGrab::None;
        _loop.state.seat.force_cursor = None;
    } else {
        _loop.inner.canvas_mut().Grab = CanvasGrab::Active(ActiveOption::Hand);
        _loop.state.seat.force_cursor = Some(CursorIcon::Grab);
        // The hand tool breaks out of any pointer lock/confine the focused client holds.
        crate::constraint::break_constraint(_loop);
    }
}

/// Open the sticky touch pane ("touch menu") in the active world so the pen can
/// operate it (feature parity with touch).
fn open_touch_menu(_loop: &mut Loop) {
    let world = _loop.inner.worlds.spawn_target().as_u128();
    _loop.inner.touch.pane_world = Some(world);
}

/// Run a button-bound action for one press/release edge.
pub fn execute(_loop: &mut Loop, action: &PenAction, pressed: bool, time: u32) {
    match action {
        PenAction::ToggleHandMode => {
            if pressed {
                toggle_hand(_loop);
            }
        }
        PenAction::OpenTouchMenu => {
            if pressed {
                open_touch_menu(_loop);
            }
        }
        PenAction::Click(btn) => crate::touch::emulate::button(_loop, btn.code(), pressed, time),
        PenAction::Key(bind) => inject::combo(_loop, bind, pressed, time),
        // Not button actions: passthrough is the caller's job; zoom/wheel are the dial's.
        PenAction::Passthrough | PenAction::Zoom | PenAction::Wheel { .. } => {}
    }
}
