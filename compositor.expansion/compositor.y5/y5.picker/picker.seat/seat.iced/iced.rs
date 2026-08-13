//! Route pointer + keyboard to the picker's details panel (iced).
//!
//! Everything resolves through the PICKER world's own registry and hit-tests in
//! SCREEN space with an identity camera — the basis `scene.frame` renders it on.
//! No focus accessor is involved: the session world's camera, registry and spawn
//! target say nothing about a panel the picker owns and draws itself.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keysym;
use smithay::utils::{Physical, Point, Size};
use compositor_monitor_compositor_iced_base::{HandleId, Transform};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{registry, PICKER_MUT, PICKER_WORLD};

/// Composed at 1:1 with no camera, so hit-testing is identity against the output.
/// Read straight off the output, not via `size_ctx_all()` — that folds in
/// `camera()`, i.e. the SPAWN TARGET's camera, which says nothing about a panel
/// the picker draws itself.
fn screen(state: &Loop) -> (Transform, Size<f64, Physical>) {
    let s = state.inner.current_output().current_mode().map(|m| m.size).unwrap_or_default();
    (Transform { zoom: 1.0, position: Point::new(0.0, 0.0) }, Size::from((s.w as f64, s.h as f64)))
}

/// The picker's own tracked pointer, screen-physical.
fn at(state: &mut Loop) -> Point<f64, Physical> {
    let p = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    Point::from(p.active.as_ref().map(|a| a.pointer).unwrap_or((0.0, 0.0)))
}

/// The panel under the picker pointer, if any.
fn hit(state: &mut Loop) -> Option<HandleId> {
    let (transform, output) = screen(state);
    let point = at(state);
    registry(&mut state.inner.worlds)?.hit_test(point, &transform, output)
}

/// Route a left-button press/release to the panel if the pointer is over it.
/// Returns true if handled (caller skips sphere logic). Off-panel press defocuses.
pub fn route_button(state: &mut Loop, code: u32, pressed: bool) -> bool {
    let target = hit(state);
    let Some(reg) = registry(&mut state.inner.worlds) else { return false };
    if pressed {
        reg.set_keyboard_focus(target);
    }
    let Some(h) = target else { return false };
    reg.dispatch_button(Some(h), code, pressed);
    true
}

/// Move the pointer over the panel (enter / motion / leave), in screen space.
pub fn route_motion(state: &mut Loop, point: Point<f64, Physical>) {
    let (transform, output) = screen(state);
    if let Some(reg) = registry(&mut state.inner.worlds) {
        reg.dispatch_pointer_at(point, &transform, output);
    }
}

/// Over the panel? For callers that must not ALSO read the click as a globe
/// click — the panel overlaps the sphere's silhouette.
pub fn over_panel(state: &mut Loop) -> bool {
    hit(state).is_some()
}

/// Route a key to the focused panel field (text editing). Returns true if the
/// panel has focus (caller skips cell navigation). Escape defocuses it.
///
/// The registry is the picker's own, so anything focused in it IS ours; `get`
/// still filters a handle that outlived its surface.
pub fn route_key(state: &mut Loop, keysym: Keysym, key_state: KeyState) -> bool {
    let raw = keysym.raw();
    let utf8 = keysym.key_char().map(|c| c.to_string());
    let pressed = matches!(key_state, KeyState::Pressed);
    let Some(reg) = registry(&mut state.inner.worlds) else { return false };
    let Some(focused) = reg.keyboard_focus().filter(|id| reg.get(*id).is_some()) else {
        return false;
    };
    if raw == smithay::input::keyboard::keysyms::KEY_Escape {
        reg.set_keyboard_focus(None);
        return true;
    }
    if let Some(m) = compositor_monitor_compositor_iced_base::input::keysym_to_iced_modifier(raw) {
        reg.modifier_changed(m, pressed);
        return true;
    }
    let eff = reg.effective_modifiers();
    if let Some(e) = compositor_monitor_compositor_iced_base::registry::translate_keyboard(
        raw, utf8.as_deref(), key_state, eff, false,
    ) {
        let _ = reg.dispatch_event(focused, e);
    }
    true
}
