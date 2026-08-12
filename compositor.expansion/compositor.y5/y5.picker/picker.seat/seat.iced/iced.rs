//! Route pointer clicks + keyboard to the picker's details panel (iced) when the
//! pointer is over it / it has focus — so its buttons + name field work. Mirrors
//! the lock screen's iced routing.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keysym;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_y5_surface_interface_base::hit::{surface_under_filtered, SurfaceHit};

/// Route a left-button press/release to the panel if the pointer is over it.
/// Returns true if handled (caller skips sphere logic). Off-panel press defocuses.
pub fn route_button(state: &mut Loop, code: u32, pressed: bool) -> bool {
    let Some(pos) = state.state.seat.seat.get_pointer().map(|p| p.current_location()) else {
        return false;
    };
    let under = surface_under_filtered(state, pos, &|hit| {
        hit.iced_layer().map(|l| (l & Layer::PICKER_SCENE.bits()) != 0).unwrap_or(false)
    });
    let handle = match under {
        Some(SurfaceHit::Iced { handle, .. }) => Some(handle),
        _ => None,
    };
    let Some(reg) = state.inner.surface_mut().registry.as_mut() else {
        return false;
    };
    match handle {
        Some(h) => {
            if pressed {
                reg.set_keyboard_focus(Some(h));
            }
            reg.dispatch_button(Some(h), code, pressed);
            true
        }
        None => {
            if pressed {
                reg.set_keyboard_focus(None);
            }
            false
        }
    }
}

/// The focused surface, but only when it is one of the PICKER's own.
///
/// The registry is the spawn-target world's and is shared with everything that
/// world draws, so whatever held keyboard focus before the picker opened — a
/// guide popup, the settings window, a launcher tile — is still recorded in it.
/// Treating that as "the panel has focus" sent the picker's own arrows, Enter and
/// Super+K to an off-screen surface, which is why the picker looked dead to the
/// keyboard until a click or Escape happened to clear the stale focus.
fn panel_focus(state: &Loop) -> Option<compositor_monitor_compositor_iced_base::HandleId> {
    let reg = state.inner.surface().registry.as_ref()?;
    reg.keyboard_focus()
        .filter(|id| reg.get(*id).is_some_and(|i| (i.layer & Layer::PICKER_SCENE.bits()) != 0))
}

/// Route a key to the focused panel field (text editing). Returns true if the
/// panel has focus (caller skips cell navigation). Escape defocuses it.
pub fn route_key(state: &mut Loop, keysym: Keysym, key_state: KeyState) -> bool {
    if panel_focus(state).is_none() {
        return false;
    }
    if keysym.raw() == smithay::input::keyboard::keysyms::KEY_Escape {
        if let Some(r) = state.inner.surface_mut().registry.as_mut() {
            r.set_keyboard_focus(None);
        }
        return true;
    }
    let raw = keysym.raw();
    let utf8 = keysym.key_char().map(|c| c.to_string());
    let pressed = matches!(key_state, KeyState::Pressed);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        if let Some(m) = compositor_monitor_compositor_iced_base::input::keysym_to_iced_modifier(raw) {
            reg.modifier_changed(m, pressed);
            return true;
        }
        let eff = reg.effective_modifiers();
        if let Some(focused) = reg.keyboard_focus() {
            if let Some(e) = compositor_monitor_compositor_iced_base::registry::translate_keyboard(
                raw,
                utf8.as_deref(),
                key_state,
                eff,
                false,
            ) {
                let _ = reg.dispatch_event(focused, e);
            }
        }
    }
    true
}
