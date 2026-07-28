//! Synthetic input injection into the focused client, for pen/pad action bindings:
//! keyboard combos (bound buttons) and a modifier + mouse-wheel scroll (the dial
//! remap for apps without tablet-v2, e.g. `Alt`+Wheel for brush size). Built on the
//! same seat primitives real input uses — `KeyboardHandle::input` (processed through
//! xkb, so held modifiers register) and `PointerHandle::axis` — so modifier state and
//! client focus stay consistent.

use smithay::backend::input::{Axis, AxisSource, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::input::pointer::AxisFrame;
use smithay::utils::SERIAL_COUNTER;
use compositor_model_environment_preference_base::base::KeyBind;
use compositor_orchestration_core_state_base::Loop;

/// Forward one synthetic key edge to the focused client. `code` is a keyboard keycode
/// as delivered to the seat (xkb = evdev + 8), captured verbatim by the binding UI.
fn key(_loop: &mut Loop, code: u32, pressed: bool, time: u32) {
    let Some(keyboard) = _loop.state.seat.seat.get_keyboard() else { return };
    let serial = SERIAL_COUNTER.next_serial();
    let st = if pressed { KeyState::Pressed } else { KeyState::Released };
    let _ = keyboard.input::<(), _>(&mut _loop.state, Keycode::new(code), st, serial, time, |_, _, _| {
        FilterResult::Forward
    });
}

/// Inject a captured keyboard combo. On the trigger's press edge, press the modifiers
/// then the key; a `hold` bind releases on the trigger's release edge, a tap releases
/// immediately (modifiers released in reverse order).
pub fn combo(_loop: &mut Loop, bind: &KeyBind, edge_pressed: bool, time: u32) {
    if bind.key == 0 {
        return;
    }
    if edge_pressed {
        for &m in &bind.mods {
            key(_loop, m, true, time);
        }
        key(_loop, bind.key, true, time);
        if !bind.hold {
            key(_loop, bind.key, false, time);
            for &m in bind.mods.iter().rev() {
                key(_loop, m, false, time);
            }
        }
    } else if bind.hold {
        key(_loop, bind.key, false, time);
        for &m in bind.mods.iter().rev() {
            key(_loop, m, false, time);
        }
    }
}

/// Inject a mouse-wheel scroll (optionally with held modifier keycodes) to the client
/// under the pointer — the dial remap. `v120` is the dial's high-res delta (120 per
/// detent); one detent maps to one wheel notch.
pub fn wheel(_loop: &mut Loop, mods: &[u32], v120: i32, time: u32) {
    for &m in mods {
        key(_loop, m, true, time);
    }
    if let Some(pointer) = _loop.state.seat.seat.get_pointer() {
        let value = (v120 as f64 / 120.0) * 15.0;
        let frame = AxisFrame::new(time)
            .source(AxisSource::Wheel)
            .value(Axis::Vertical, value)
            .v120(Axis::Vertical, v120);
        pointer.axis(&mut _loop.state, frame);
        pointer.frame(&mut _loop.state);
    }
    for &m in mods.iter().rev() {
        key(_loop, m, false, time);
    }
}
