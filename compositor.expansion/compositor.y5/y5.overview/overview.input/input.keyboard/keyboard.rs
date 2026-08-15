//! Overview keyboard handling. The canvas router calls `handle` first; returning
//! true INTERCEPTs (see canvas `input.rs`). While open the overview consumes ALL
//! keys: Super+Tab toggles, Super+Left/Right cycle tabs, the World globe takes
//! arrows/Enter (unless its details panel has focus), Escape closes, everything
//! else goes to the overview's own screen iced surfaces — never a client window.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::{Keysym, ModifiersState};
use compositor_monitor_compositor_iced_base::IcedSpace;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_library_input_keyboard_base::keyboard::key::Key;

pub fn handle(
    key: Option<Key>,
    keysym: Keysym,
    key_state: KeyState,
    modifiers: &ModifiersState,
    state: &mut Loop,
) -> bool {
    let press = key_state == KeyState::Pressed;
    // Super. In a nested session Right Ctrl is substituted for it at the input
    // (`seat.keyboard/keyboard.input::shortcut_modifiers`), so there is nothing
    // to special-case here — and Left Ctrl stays a real Ctrl for the clients.
    let modkey = modifiers.logo;

    // Super+Tab toggles the overlay from any state.
    if press && modkey && key == Some(Key::Tab) {
        compositor_y5_overview_interface_base::base::toggle(state);
        return true;
    }
    if !state.inner.overview().visible {
        return false;
    }
    // World tab: the panel's name field takes the key whenever it holds focus, or
    // the globe navigation below swallows every keystroke and rename does nothing.
    // Both edges (modifier tracking); modkey combos stay with the tab/globe binds.
    if !modkey && state.inner.overview().is_world()
        && compositor_y5_picker_seat_iced::iced::route_key(state, keysym, key_state) {
        return true;
    }
    if press {
        // Escape dismisses the logout popup first (before closing the overlay).
        if key == Some(Key::Escape) && state.inner.overview().logout.is_some() {
            compositor_y5_overview_interface_base::base::toggle_logout(state);
            return true;
        }
        // Super+Left/Right cycle the tabs in any tab.
        if modkey && matches!(key, Some(Key::Left) | Some(Key::Right)) {
            compositor_y5_overview_interface_surface::surface::cycle_tab(state, key == Some(Key::Right));
            return true;
        }
        if state.inner.overview().is_world() {
            match key {
                Some(Key::Left) => compositor_y5_picker_interface_embed::embed::select_direction(state, -1, 0),
                Some(Key::Right) => compositor_y5_picker_interface_embed::embed::select_direction(state, 1, 0),
                Some(Key::Up) => compositor_y5_picker_interface_embed::embed::select_direction(state, 0, 1),
                Some(Key::Down) => compositor_y5_picker_interface_embed::embed::select_direction(state, 0, -1),
                Some(Key::Return) => compositor_y5_overview_interface_activate::activate::activate_world(state),
                Some(Key::Escape) => { compositor_y5_overview_interface_base::base::request_close(state); }
                _ => {}
            }
            return true;
        }
        if key == Some(Key::Escape) {
            compositor_y5_overview_interface_base::base::request_close(state);
            return true;
        }
    }
    // Non-world tabs (e.g. Settings): the focused screen iced surface. Press AND
    // release; the overlay owns the key either way.
    route_screen_iced(state, keysym, key_state);
    true
}

/// Forward a key to the focused SCREEN-space iced surface (menu bar / settings).
fn route_screen_iced(state: &mut Loop, keysym: Keysym, key_state: KeyState) {
    let Some(reg) = state.inner.surface_mut().registry.as_mut() else { return };
    let Some(focus) = reg.keyboard_focus() else { return };
    if reg.space_of(focus) != Some(IcedSpace::Screen) {
        return;
    }
    let raw = keysym.raw();
    let pressed = matches!(key_state, KeyState::Pressed);
    if let Some(m) = compositor_monitor_compositor_iced_base::input::keysym_to_iced_modifier(raw) {
        reg.modifier_changed(m, pressed);
        return;
    }
    let eff = reg.effective_modifiers();
    let utf8 = keysym.key_char().map(|c| c.to_string());
    if let Some(e) = compositor_monitor_compositor_iced_base::registry::translate_keyboard(
        raw, utf8.as_deref(), key_state, eff, false,
    ) {
        let _ = reg.dispatch_event(focus, e);
    }
}
