use smithay::backend::input::{
    Axis, AxisSource, Event, InputBackend, KeyState, KeyboardKeyEvent, PointerAxisEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, Keysym};
use smithay::input::pointer::{AxisFrame, PointerHandle};
use smithay::utils::SERIAL_COUNTER;
use compositor_orchestration_core_state_base::Loop;

/// Release every currently-held **non-modifier** key to the focused client (forwarded, no
/// intercept), to be called right before keyboard focus is cleared. Some clients track their own
/// keyboard state and don't reset on `wl_keyboard.leave`, so they resume in a key-down state when
/// they regain focus (the "stuck key" bug); the explicit releases clear that. Modifiers are
/// deliberately skipped so neither the compositor's nor the app's modifier tracking is desynced.
pub fn release_held_keys(loop_: &mut Loop) {
    let Some(keyboard) = loop_.state.seat.seat.get_keyboard() else {
        return;
    };
    // smithay seat callback now sees `&mut Dispatch` (D = Dispatch).
    // Collect the non-modifier held keycodes (the lock is released before we re-enter `input`).
    let to_release: Vec<Keycode> = keyboard.with_pressed_keysyms(|syms| {
        syms.iter()
            .filter(|h| !is_modifier_keysym(h.modified_sym().raw()))
            .map(|h| h.raw_code())
            .collect()
    });
    if to_release.is_empty() {
        return;
    }
    let time = loop_.inner.start_time.elapsed().as_millis() as u32;
    for key in to_release {
        let serial = SERIAL_COUNTER.next_serial();
        let _ = keyboard.input::<(), _>(&mut loop_.state, key, KeyState::Released, serial, time, |_, _, _| {
            FilterResult::Forward
        });
    }
}

/// X11 keysym ranges for modifier keys: `0xffe1..=0xffee` (Shift/Control/Caps/Meta/Alt/Super/
/// Hyper), `0xff7f` (Num_Lock), `0xfe01..=0xfe13` (ISO level shifts / AltGr). See `<keysymdef.h>`.
fn is_modifier_keysym(raw: u32) -> bool {
    matches!(raw, 0xffe1..=0xffee | 0xff7f | 0xfe01..=0xfe13)
}

pub fn input_received<I: InputBackend>(event: &I::KeyboardKeyEvent, _loop: &mut Loop) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = Event::time_msec(event);
    let key_state = event.state();
    let key_code = event.key_code();

    {
        // World input bus first (phase 3); Pass falls through to legacy routing.
        // Modifiers transitional 0 until the first keyboard-consuming system lands.
        let ev = compositor_support_system_input_event_base::base::InputEvent::Keyboard {
            code: key_code.raw(),
            pressed: key_state == smithay::backend::input::KeyState::Pressed,
            modifiers: 0,
        };
        if compositor_orchestration_input_drive_base::drive::route(_loop, ev)
            == compositor_support_system_input_event_base::base::InputFlow::Consume
        {
            return;
        }
    }

    // D = Dispatch (the seat callback is world-free). The world-touching keyboard
    // routing below needs the whole `Loop` (shortcut actions are `Fn(&mut Loop)`),
    // so we use smithay's documented async-decide pattern: `input_intercept`
    // processes the key through xkb ONCE and yields the modified keysym WITHOUT
    // forwarding; we then decide intercept-vs-forward with the full `Loop`; and
    // `input_forward` sends the (already-processed) key to the focused client only
    // if nothing intercepted (no xkb re-processing, no double-count). This restores
    // the pre-P2 intercept semantics — shortcuts no longer leak to the focused
    // client. (P3, document/SMITHAY_DECOUPLING.md.)
    let keyboard = _loop.state.seat.seat.get_keyboard().unwrap();
    let ((keysym, modifiers), mods_changed) = keyboard.input_intercept(
        &mut _loop.state,
        key_code,
        key_state,
        |_d, modifiers, handle| (handle.modified_sym(), *modifiers),
    );

    if should_forward::<I>(_loop, keysym, key_state, &modifiers) {
        keyboard.input_forward(&mut _loop.state, key_code, key_state, serial, time, mods_changed);
    }
}

/// Run the world keyboard routing (overlay shortcuts → launcher/iced → canvas
/// shortcuts → wayland focus → iced) with the full `Loop`, returning whether the
/// key should be FORWARDED to the focused client (`true`) or was intercepted
/// (`false`). Mirrors the pre-P2 in-callback `FilterResult` decision exactly.
fn should_forward<I: InputBackend>(
    _loop: &mut Loop,
    keysym: Keysym,
    key_state: KeyState,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> bool {
    if compositor_y5_overlay_interface_keyboard::keyboard::input_received::<I>(
        _loop, keysym, key_state, modifiers,
    ) {
        return false; // overlay shortcut consumed it
    }

    // While DARK (no visible output — DPMS-off / lid-closed / all monitors gone),
    // stop after the fixed/overlay shortcuts above: canvas/navigator shortcuts and
    // forwarding to the focused client are suppressed (there's nothing on screen to
    // drive). EXCEPTION: an output change mid-provisioning (an Apply awaiting the
    // user's Keep/Revert) shows its countdown dialog in the settings window — keep
    // feeding keys to iced so the user can confirm/revert blind, otherwise the only
    // way out of a bad (black) mode is to wait for the auto-revert timeout.
    if is_dark(_loop) {
        if output_provisioning(_loop) {
            if let Some(result) = iced_handle(_loop, keysym, key_state) {
                return !result;
            }
        }
        return false; // intercept everything else while dark
    }

    let screen_handler =
        compositor_y5_launcher_input_base::keyboard::keyboard_received(key_state, modifiers, _loop)
            .is_none();
    if screen_handler {
        if let Some(result) = iced_handle(_loop, keysym, key_state) {
            return !result; // iced consumed (true) → intercept; else forward
        }
    }

    let shortcut_intercept =
        compositor_y5_canvas_input_keyboard::keyboard::input_received(key_state, modifiers, keysym, _loop)
            .is_none();
    if shortcut_intercept {
        return false;
    }

    if let Some(result) = wayland_handle(_loop) {
        return !result;
    }
    if let Some(result) = iced_handle(_loop, keysym, key_state) {
        return !result;
    }
    true
}

/// The compositor currently has no visible output (DPMS-off, lid closed, or every
/// monitor unplugged). Set/cleared by the display apply/switch paths.
fn is_dark(_loop: &Loop) -> bool {
    *_loop
        .inner
        .kernel
        .get(&compositor_orchestration_driver_lid_base::base::DISPLAY_OFF)
}

/// An output mode / preferred-monitor change is provisionally applied and awaiting
/// the user's Keep/Revert (the settings window shows the countdown dialog). True
/// only inside that confirm window — cleared once the transaction confirms/reverts.
fn output_provisioning(_loop: &Loop) -> bool {
    use compositor_orchestration_driver_output_base::base::{ApplyResult, OUTPUT_MODE_RESULT};
    // Multi-output branch: only the per-pipe mode change has a provisional confirm gate;
    // the single-output active-switch transaction (OUTPUT_SWITCH_RESULT) was removed.
    matches!(
        _loop.inner.kernel.get(&OUTPUT_MODE_RESULT),
        Some(ApplyResult::Provisional)
    )
}

fn wayland_handle(state: &mut Loop) -> Option<bool> {
    let keyboard = state.state.seat.seat.get_keyboard().unwrap();
    if keyboard.is_focused() {
        return Some(false);
    }
    None
}

fn iced_handle(state: &mut Loop, keysym: Keysym, key_state: KeyState) -> Option<bool> {
    // This was moved before wayland. It shouldn't matter unless the prior screen space logic is being used.
    if let Some(registry) = state.inner.surface_mut().registry.as_mut() {
        let keysym_raw = keysym.raw();
        let utf8 = keysym.key_char().map(|c| c.to_string());
        let pressed = matches!(key_state, KeyState::Pressed);

        // If this key is a modifier itself, update tracked state and stop.
        if let Some(mod_bit) = compositor_monitor_compositor_iced_base::input::keysym_to_iced_modifier(keysym_raw)
        {
            registry.modifier_changed(mod_bit, pressed);
            return Some(true);
        };

        // Non-modifier key: route to iced with effective modifier state.
        if let Some(focused) = registry.keyboard_focus() {
            let effective = registry.effective_modifiers();
            if let Some(e) = compositor_monitor_compositor_iced_base::registry::translate_keyboard(
                keysym_raw,
                utf8.as_deref(),
                key_state,
                effective,
                false,
            ) {
                let _ = registry.dispatch_event(focused, e);
            }
            return Some(true);
        }
    }
    None
}
// To add keyboard events, inside the courier function:
// BEGIN - Keyboard navigation, currently disabled.
//
// if key_state == KeyState::Pressed {
//     let keysym = handle.modified_sym();
//     let speed = 100.0 / state.zoom;
//
//     // Match against the constants in smithay::input::keyboard::keysyms
//     if keysym == keysyms::KEY_Left.into() {
//         state.camera_pos.x -= speed;
//         return FilterResult::Intercept(());
//     } else if keysym == keysyms::KEY_Right.into() {
//         state.camera_pos.x += speed;
//         return FilterResult::Intercept(());
//     } else if keysym == keysyms::KEY_Up.into() {
//         state.camera_pos.y -= speed;
//         return FilterResult::Intercept(());
//     } else if keysym == keysyms::KEY_Down.into() {
//         state.camera_pos.y += speed;
//         return FilterResult::Intercept(());
//     } else if keysym == keysyms::KEY_plus.into() || keysym == keysyms::KEY_equal.into() || keysym == keysyms::KEY_KP_Add.into() {
//         state.zoom *= 1.1;
//         return FilterResult::Intercept(());
//     } else if keysym == keysyms::KEY_minus.into() || keysym == keysyms::KEY_KP_Subtract.into() {
//         state.zoom /= 1.1;
//         return FilterResult::Intercept(());
//     }
// }
// END - Keyboard navigation, currently disabled.
