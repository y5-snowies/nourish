//! Picker keyboard input: arrow keys move the focus, Enter starts the focused
//! cell's world, Escape / Super+K cancel.

use smithay::backend::input::{Event, InputBackend, KeyState, KeyboardKeyEvent};
use smithay::input::keyboard::{FilterResult, Keysym, ModifiersState};
use smithay::utils::SERIAL_COUNTER;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_library_input_keyboard_base::keyboard::key::Key;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

pub fn input_received<I: InputBackend>(event: &I::KeyboardKeyEvent, state: &mut Loop) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = Event::time_msec(event);
    let key_state = event.state();
    let key_code = event.key_code();

    // Extract the keysym + modifiers via the seat (the callback only sees
    // `&mut Dispatch`), then act with `state` free. No client windows here, so
    // always intercept at the wayland level.
    // `keysym` is the active-layout sym (what the details field types);
    // `shortcut_sym` the layout-agnostic Latin identity of the physical key, as
    // the main path uses for matching — with an `il`-style layout, Alt+Shift
    // toggles layouts and the modified sym is no longer a Latin letter.
    let mut extracted: Option<(Keysym, Keysym, ModifiersState)> = None;
    state.state.seat.seat.get_keyboard().unwrap().input::<(), _>(
        &mut state.state,
        key_code,
        key_state,
        serial,
        time,
        |_state: &mut Dispatch, modifiers, handle| {
            let modified = handle.modified_sym();
            let shortcut = handle.raw_latin_sym_or_raw_current_sym().unwrap_or(modified);
            extracted = Some((modified, shortcut, *modifiers));
            FilterResult::Intercept(())
        },
    );

    let Some((keysym, shortcut_sym, modifiers)) = extracted else {
        return;
    };
    // Nested session: Right Ctrl stands in for Super (the host eats the real
    // one) — the same substitution the main path's matchers see.
    let (shortcut_sym, modifiers) =
        compositor_orchestration_seat_keyboard_input::keyboard::nested_shortcut_view(state, shortcut_sym, modifiers);
    // Overlay shortcuts (sleep, diagnostics, ...) apply in the picker as on the
    // lock screen. A Super chord is never text, so it goes first even while the
    // details field has focus; everything else reaches the field first.
    let overlay = |state: &mut Loop| {
        compositor_y5_overlay_interface_keyboard::keyboard::input_received::<I>(state, shortcut_sym, key_state, &modifiers)
    };
    if modifiers.logo && overlay(state) {
        return;
    }
    // If the details panel field has focus, the key edits it (Esc defocuses).
    if compositor_y5_picker_seat_iced::iced::route_key(state, keysym, key_state) {
        return;
    }
    if !modifiers.logo && overlay(state) {
        return;
    }
    if key_state != KeyState::Pressed {
        return;
    }

    match Key::from_keysym(shortcut_sym) {
        Some(Key::Left) => navigate(state, -1, 0),
        Some(Key::Right) => navigate(state, 1, 0),
        Some(Key::Up) => navigate(state, 0, 1),
        Some(Key::Down) => navigate(state, 0, -1),
        Some(Key::Return) => compositor_y5_picker_world_base::base::start(state),
        Some(Key::Escape) => compositor_y5_picker_interface_base::base::cancel(state),
        Some(Key::K) if modifiers.logo => compositor_y5_picker_interface_base::base::cancel(state),
        _ => {}
    }
}

/// Move the focused cell one step in a SCREEN direction (`du`: +right/-left,
/// `dv`: +up/-down as drawn), then animate to face it.
///
/// Stepped against `target`, not the live orientation: a chain of arrow presses
/// would otherwise each measure a sphere still gliding toward the last one, and
/// the same key would land differently depending on how fast it was pressed.
fn navigate(state: &mut Loop, du: i32, dv: i32) {
    let (current, target) = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_ref()
        .map(|a| (a.selected.unwrap_or(0), a.target))
        .unwrap_or_default();
    let next = compositor_y5_picker_pick_navigate::navigate::neighbor(current, du, dv, target);
    compositor_y5_picker_command_base::base::set_selected(state, Some(next));
}
