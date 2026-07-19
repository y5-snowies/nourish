//! Compositor-side handling of OSK taps (drained from the surface channel by the
//! global pump). Injects synthesized key edges into the focused client / iced
//! surface via `keyboard::inject_key`, tracks the sticky modifiers, and clears
//! everything on close so no modifier stays stuck for the physical keyboard.
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_keyboard_input::keyboard::inject_key;
use compositor_y5_osk_board_state::state::{OSK, OSK_MUT, OskMods};
use compositor_y5_osk_board_view::view::{ModKind, OskMessage};
use smithay::input::keyboard::Keycode;
use smithay::wayland::text_input::TextInputSeat;

/// Frames a tapped key stays highlighted (~100ms at 60fps).
const FLASH_FRAMES: u8 = 6;

/// Left-modifier xkb keycodes (evdev + 8).
const KC_CTRL: u32 = 37;
const KC_ALT: u32 = 64;
const KC_SHIFT: u32 = 50;
const KC_LOGO: u32 = 133;

fn now(state: &Loop) -> u32 {
    state.inner.start_time.elapsed().as_millis() as u32
}

pub fn delegate(state: &mut Loop, message: OskMessage) {
    match message {
        OskMessage::Key(kc) => type_key(state, kc),
        OskMessage::Mod(m) => toggle_mod(state, m),
        OskMessage::Globe => cycle_layout(state),
        OskMessage::Close => close(state),
        // compositor → surface only
        OskMessage::SetMods { .. }
        | OskMessage::SetLabels(_)
        | OskMessage::SetPreview(_)
        | OskMessage::SetFlash(_) => {}
    }
}

/// Inject one key: press the toggled sticky modifiers, tap the key, release the
/// modifiers (reverse order), then clear the one-shot sticky state. The keycode is
/// resolved by the live xkb layout inside `inject_key`, so it types the right char.
fn type_key(state: &mut Loop, kc: u32) {
    let time = now(state);
    let mods = state.inner.kernel.get(&OSK).mods;
    let mut mod_kcs: Vec<u32> = Vec::new();
    if mods.ctrl {
        mod_kcs.push(KC_CTRL);
    }
    if mods.alt {
        mod_kcs.push(KC_ALT);
    }
    if mods.shift {
        mod_kcs.push(KC_SHIFT);
    }
    if mods.logo {
        mod_kcs.push(KC_LOGO);
    }
    for &m in &mod_kcs {
        inject_key(state, Keycode::new(m), true, time);
    }
    inject_key(state, Keycode::new(kc), true, time);
    inject_key(state, Keycode::new(kc), false, time);
    for &m in mod_kcs.iter().rev() {
        inject_key(state, Keycode::new(m), false, time);
    }
    if mods.any() {
        state.inner.kernel.get_mut(&OSK_MUT).mods = OskMods::default();
    }
    // Local echo for the preview (skip sensitive fields so a password is never echoed).
    if !state.state.seat.seat.text_input().is_sensitive_field() {
        update_echo(state, kc, mods.shift);
    }
    // Visible tap feedback: flash this key highlighted for a few frames. Input schedules
    // no redraw on its own, so kick one now; the reconciler pumps the rest.
    {
        let st = state.inner.kernel.get_mut(&OSK_MUT);
        st.flash_key = Some(kc);
        st.flash_frames = FLASH_FRAMES;
    }
    state.schedule_redraw();
}

/// Update the OSK's local echo buffer from a tapped keycode: backspace deletes, Enter
/// clears (submit), space inserts a space, any other key appends its character.
fn update_echo(state: &mut Loop, kc: u32, shift: bool) {
    match kc {
        22 => {
            state.inner.kernel.get_mut(&OSK_MUT).echo.pop();
        }
        36 => {
            state.inner.kernel.get_mut(&OSK_MUT).echo.clear();
        }
        65 => {
            state.inner.kernel.get_mut(&OSK_MUT).echo.push(' ');
        }
        _ => {
            if let Some(c) = base_char(state, kc) {
                let c = if shift { c.to_uppercase().next().unwrap_or(c) } else { c };
                state.inner.kernel.get_mut(&OSK_MUT).echo.push(c);
            }
        }
    }
}

/// The unshifted character a keycode produces in the active xkb layout (echo source).
fn base_char(state: &mut Loop, kc: u32) -> Option<char> {
    let kb = state.state.seat.seat.get_keyboard()?;
    kb.with_xkb_state(&mut state.state, |ctx| {
        let xkb = ctx.xkb().lock().unwrap();
        let layout = xkb.active_layout();
        xkb.raw_syms_for_key_in_layout(Keycode::new(kc), layout)
            .first()
            .and_then(|s| s.key_char())
    })
}

/// Globe key: cycle the seat's active XKB layout (the configured languages). Injected
/// keycodes resolve through the live layout, so subsequent keys type the new language;
/// a physical keyboard follows the same layout.
fn cycle_layout(state: &mut Loop) {
    if let Some(kb) = state.state.seat.seat.get_keyboard() {
        kb.with_xkb_state(&mut state.state, |mut ctx| {
            ctx.cycle_next_layout();
        });
    }
}

fn toggle_mod(state: &mut Loop, kind: ModKind) {
    let m = &mut state.inner.kernel.get_mut(&OSK_MUT).mods;
    match kind {
        ModKind::Shift => m.shift = !m.shift,
        ModKind::Ctrl => m.ctrl = !m.ctrl,
        ModKind::Alt => m.alt = !m.alt,
        ModKind::Logo => m.logo = !m.logo,
    }
}

/// Dismiss the OSK: release any modifier keycode still injected-down (so nothing
/// stays stuck for the physical keyboard), clear sticky state, and hide.
pub fn close(state: &mut Loop) {
    let time = now(state);
    let held: Vec<u32> = state.inner.kernel.get(&OSK).held.clone();
    for m in held {
        inject_key(state, Keycode::new(m), false, time);
    }
    let st = state.inner.kernel.get_mut(&OSK_MUT);
    st.held.clear();
    st.mods = OskMods::default();
    st.open = false;
    st.pinned = false;
    st.echo.clear();
    // Suppress auto-show until the focused field blurs, so ✕ actually hides it.
    st.dismissed = true;
}
