use crate::keyboard::iced;
use smithay::backend::input::{
    Axis, AxisSource, Event, InputBackend, KeyState, KeyboardKeyEvent, PointerAxisEvent,
};
use smithay::input::keyboard::{FilterResult, Keysym, ModifiersState};
use smithay::input::pointer::{AxisFrame, PointerHandle};
use smithay::utils::SERIAL_COUNTER;
use compositor_orchestration_core_state_base::Loop;

pub fn input_received<I: InputBackend>(
    state: &mut Loop,
    keysym: Keysym,
    key_state: KeyState,
    modifiers: &ModifiersState,
) -> bool {
    let input_handle = {
        let active = &mut state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active;
        let active = active.as_ref().unwrap_or_else(|| abort!("is locked"));

        let Some(input_handle) = active.surface_input else {
            return false;
        };

        input_handle.id.clone()
    };

    let Some(registry) = compositor_y5_lock_system_base::base::registry(&mut state.inner.worlds) else {
        return false;
    };

    // Make sure the input handle has focus.
    registry.set_keyboard_focus(Some(input_handle));

    let keysym_raw = keysym.raw();
    let utf8 = keysym.key_char().map(|c| c.to_string());
    let pressed = matches!(key_state, KeyState::Pressed);

    // If this key is a modifier itself, update tracked state and stop.
    if let Some(mod_bit) = compositor_monitor_compositor_iced_base::input::keysym_to_iced_modifier(keysym_raw) {
        registry.modifier_changed(mod_bit, pressed);
        // Just modifier change currently doesnt interecept?
        return false;
    };

    let Some(focused) = registry.keyboard_focus() else {
        return false;
    };

    let effective = registry.effective_modifiers();
    let Some(e) = compositor_monitor_compositor_iced_base::registry::translate_keyboard(
        keysym_raw,
        utf8.as_deref(),
        key_state,
        effective,
        false,
    ) else {
        return false;
    };

    let _ = registry.dispatch_event(focused, e);

    true
}
