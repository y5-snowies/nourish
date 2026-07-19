//! `zwp_tablet_tool_v2` stylus (barrel) buttons.
//!
//! Resolution order: an explicit user binding (Settings → Pen) wins; else, in
//! below-threshold cursor mode, a button chosen as the left/right click fires that;
//! else the built-in default — native to a tablet-aware surface, otherwise the pen
//! acts as a mouse (lower barrel → right-click, upper → middle-click). A press
//! latches its emulated-button route so the matching release fires the same thing
//! even if the pen crossed a window edge while held.

use smithay::backend::input::{ButtonState, Event, InputBackend, TabletToolButtonEvent, TabletToolEvent};
use smithay::utils::SERIAL_COUNTER;
use compositor_developer_environment_preference_base::base::PenAction;
use compositor_orchestration_core_state_base::Loop;
use crate::tablet::action;
use crate::touch::backend::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};

/// linux/input-event-codes.h stylus button codes.
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

/// Default barrel-button → pointer-button map for non-tablet targets. `None` ⇒ the
/// button has no mouse emulation (left inert off a tablet client).
fn map_stylus_button(raw: u32) -> Option<u32> {
    match raw {
        BTN_STYLUS => Some(BTN_RIGHT),
        BTN_STYLUS2 => Some(BTN_MIDDLE),
        _ => None,
    }
}

pub fn button<I: InputBackend>(event: &I::TabletToolButtonEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let raw = event.button();
    let state = event.button_state();
    let pressed = state == ButtonState::Pressed;
    let time = event.time_msec();

    // 1. An explicit binding (Settings → Pen) overrides everything.
    let bound = _loop.inner.preference.pen.stylus_action(raw);
    if !matches!(bound, PenAction::Passthrough) {
        action::execute(_loop, &bound, pressed, time);
        return;
    }

    // 2. Light-touch mode: the chosen in-pen buttons are the click buttons.
    let pen = &_loop.inner.preference.pen;
    if pen.below_threshold_cursor {
        if pen.below_left == Some(raw) {
            crate::touch::emulate::button(_loop, BTN_LEFT, pressed, time);
            return;
        }
        if pen.below_right == Some(raw) {
            crate::touch::emulate::button(_loop, BTN_RIGHT, pressed, time);
            return;
        }
    }

    // 3. Built-in default.
    match state {
        ButtonState::Pressed => {
            if _loop.state.tablet.tool_has_focus(&tool) {
                // Tablet-aware surface → native stylus button.
                let serial = SERIAL_COUNTER.next_serial();
                _loop.state.tablet.tool_button(&tool, raw, state, serial, time);
            } else if let Some(mapped) = map_stylus_button(raw) {
                // Non-tablet target → the pen is a mouse.
                crate::touch::emulate::button(_loop, mapped, true, time);
                _loop.state.tablet.push_emulated_button(raw, mapped);
            }
        }
        ButtonState::Released => {
            // A release matching an emulated press wins regardless of where the pen
            // now sits, so a held barrel button never gets stuck pressed.
            if let Some(mapped) = _loop.state.tablet.take_emulated_button(raw) {
                crate::touch::emulate::button(_loop, mapped, false, time);
            } else {
                let serial = SERIAL_COUNTER.next_serial();
                _loop.state.tablet.tool_button(&tool, raw, state, serial, time);
            }
        }
    }
}
