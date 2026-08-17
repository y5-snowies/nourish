use smithay::backend::input::{
    Axis, AxisSource, Event, InputBackend, KeyState, KeyboardKeyEvent, PointerAxisEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, Keysym};
use smithay::input::pointer::{AxisFrame, PointerHandle};
use smithay::utils::SERIAL_COUNTER;
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};
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

/// Escape keysym — cancels a pen key-bind capture.
const KEY_ESCAPE: u32 = 0xff1b;

/// `Control_L` / `Control_R`. Nested sessions treat the two as DIFFERENT
/// modifiers — see [`nested_shortcut_view`].
const KEY_CONTROL_L: u32 = 0xffe3;
const KEY_CONTROL_R: u32 = 0xffe4;
/// `Super_L` — what Right Ctrl presents AS in a nested session.
const KEY_SUPER_L: u32 = 0xffeb;

/// The keysym + modifier state the SHORTCUT matchers are asked about.
///
/// Identity on real hardware, and identity whenever Ctrl is not held at all.
///
/// Nested (winit) is the exception, and the rule there is one substitution, not a
/// remap of every binding:
///
/// * **Right Ctrl stands in for Super.** The host session owns Super and never
///   forwards it, so the compositor would otherwise have no modifier of its own.
/// * **Left Ctrl stays Ctrl.** It is what the clients running INSIDE the session
///   use — Ctrl+C, Ctrl+T, everything — and it is also what a genuinely
///   Ctrl-based COMPOSITOR binding means. Both keep working.
///
/// So a `Super+X` binding matches on RCtrl+X, a `Ctrl+X` binding matches on
/// LCtrl+X, and `Super+Ctrl+Alt` (the Hand tool) matches on RCtrl+LCtrl+Alt —
/// each without a single binding being rewritten. Substituting at the INPUT is
/// what makes that possible: the old approach rewrote every Super combo's `logo`
/// into `ctrl`, which collapsed the two meanings into one bit and left no way to
/// express a real Ctrl shortcut in a nested session.
///
/// `logo` is OR-ed rather than assigned, so a host that does forward Super (none
/// do today) would still work.
///
/// Read off the live pressed set rather than tracked on the side: the seat
/// already knows which physical keys are held, and a flag maintained here would
/// drift on any release the compositor never saw.
///
/// This is the matchers' view ONLY. Clients are forwarded the untouched xkb state
/// (`input_forward` takes `mods_changed`), so a nested client still sees a plain
/// Ctrl on either key.
fn nested_shortcut_view(
    _loop: &Loop,
    shortcut_sym: Keysym,
    modifiers: smithay::input::keyboard::ModifiersState,
) -> (Keysym, smithay::input::keyboard::ModifiersState) {
    if !_loop.inner.storage.nested {
        return (shortcut_sym, modifiers);
    }
    // The KEY identity moves with the modifier bit. `Key::from_keysym` folds both
    // controls onto `Key::Ctrl`, and `KeyCombo::matches` cross-checks the pressed
    // key against the combo's modifiers — so a modifier-only combo like the Move
    // grab (`Super` held, no key) is asked "is the key you pressed the Super this
    // combo wants?" and would answer no for a bare `Control_R`, leaving every
    // grab tool dead in a nested session. Substituting only the bits is not
    // enough; the sym has to say Super too.
    let shortcut_sym = if shortcut_sym.raw() == KEY_CONTROL_R {
        Keysym::new(KEY_SUPER_L)
    } else {
        shortcut_sym
    };
    // No Ctrl held at all ⇒ neither half is down, so the full computation below
    // would return exactly these bits. Skip the pressed-set scan.
    //
    // This is also what makes RELEASE correct: when the last Ctrl goes up, xkb has
    // already cleared `ctrl` (smithay's `key_input` removes the keycode from the
    // pressed set BEFORE the filter runs), so the early return yields
    // `logo: false, ctrl: false` — "Super released" — and a held grab cancels.
    if !modifiers.ctrl {
        return (shortcut_sym, modifiers);
    }
    let (left, right) = _loop
        .state
        .seat
        .seat
        .get_keyboard()
        .map(|keyboard| {
            keyboard.with_pressed_keysyms(|syms| {
                let raw = |h: &smithay::input::keyboard::KeysymHandle<'_>| h.modified_sym().raw();
                (
                    syms.iter().any(|h| raw(h) == KEY_CONTROL_L),
                    syms.iter().any(|h| raw(h) == KEY_CONTROL_R),
                )
            })
        })
        .unwrap_or((false, false));
    let modifiers = smithay::input::keyboard::ModifiersState {
        logo: modifiers.logo || right,
        ctrl: left,
        ..modifiers
    };
    (shortcut_sym, modifiers)
}

/// The evdev+8 (xkb) keycodes of the LEFT modifiers held, in a stable order — for a
/// captured combo, replayed by the injector (`tablet/inject.rs`).
fn modifier_keycodes(m: &smithay::input::keyboard::ModifiersState) -> Vec<u32> {
    let mut v = Vec::new();
    if m.ctrl { v.push(37); }   // LEFTCTRL 29 + 8
    if m.alt { v.push(64); }    // LEFTALT  56 + 8
    if m.shift { v.push(50); }  // LEFTSHIFT 42 + 8
    if m.logo { v.push(133); }  // LEFTMETA 125 + 8
    v
}

/// While the settings Pen tab is armed to capture a key, swallow keys and record the
/// first non-modifier press as a [`KeyBind`] (raw keycode + held modifier keycodes);
/// Escape cancels. Returns `true` when the key was consumed by capture.
fn capture_pen_key(
    _loop: &mut Loop,
    key_code: Keycode,
    keysym: Keysym,
    key_state: KeyState,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> bool {
    use compositor_orchestration_driver_settings_base::base::{PenCapture, SETTINGS, SETTINGS_MUT};
    if _loop.inner.kernel.get(&SETTINGS).pen_capture != PenCapture::Key {
        return false;
    }
    if key_state == KeyState::Pressed {
        let raw = keysym.raw();
        if raw == KEY_ESCAPE {
            _loop.inner.kernel.get_mut(&SETTINGS_MUT).pen_capture = PenCapture::None;
        } else if !is_modifier_keysym(raw) {
            let bind = compositor_model_environment_preference_base::base::KeyBind {
                mods: modifier_keycodes(modifiers),
                key: key_code.raw(),
                hold: false,
            };
            let st = _loop.inner.kernel.get_mut(&SETTINGS_MUT);
            st.pen_capture = PenCapture::None;
            st.pen_captured_key = Some(bind);
        }
    }
    true // swallow every key while capturing (no forward, no shortcuts)
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
    // Resolve TWO keysyms in one xkb pass. `keysym` = the active-layout MODIFIED sym —
    // what the user is TYPING (fed to iced text fields; clients re-resolve from the
    // keycode). `shortcut_sym` = a layout-AGNOSTIC sym (the Latin / base identity of the
    // physical key) for shortcut + pen-capture matching, so Super+N / Super+F keep
    // working when the active XKB layout is non-Latin (e.g. `il`) while text entry still
    // respects the active layout. `raw_latin_sym_or_raw_current_sym` returns the ASCII
    // sym from another layout when the active one is non-Latin, else the current sym.
    let ((keysym, shortcut_sym, modifiers), mods_changed) = keyboard.input_intercept(
        &mut _loop.state,
        key_code,
        key_state,
        |_d, modifiers, handle| {
            let modified = handle.modified_sym();
            // Default the shortcut sym to the MODIFIED sym so modifier-produced keysyms
            // keep working (e.g. Ctrl+Alt+F1 → `XF86Switch_VT_1` for the TTY switch, and
            // every ASCII binding on a Latin layout). ONLY when the active layout maps
            // the key to a NON-ASCII printable (a non-Latin letter, e.g. `il`) do we fall
            // back to the Latin/base sym, so Super+N/Super+F still match there.
            let shortcut = match modified.key_char() {
                Some(c) if !c.is_ascii() => {
                    handle.raw_latin_sym_or_raw_current_sym().unwrap_or(modified)
                }
                _ => modified,
            };
            (modified, shortcut, *modifiers)
        },
    );

    // Pen click-to-bind: while the settings Pen tab is capturing a key, swallow all
    // keys; the first non-modifier press records the combo (Escape cancels). Uses the
    // layout-agnostic sym so a captured binding matches regardless of active layout.
    if capture_pen_key(_loop, key_code, shortcut_sym, key_state, &modifiers) {
        return;
    }

    // Any keystroke dismisses an open guide popup — it is an overlay on the
    // canvas, not a mode, so the moment the user does something else it should be
    // gone. NOT swallowed: the key still does whatever it does.
    //
    // Bare modifiers are exempt on purpose. Every shortcut the help panel teaches
    // is a modifier hold (Super-drag, Super+Ctrl+Alt); dismissing on the Super
    // press would tear the panel away the instant someone tried what it just told
    // them to, before the combo was even complete.
    if key_state == KeyState::Pressed && !is_modifier_keysym(shortcut_sym.raw()) {
        compositor_y5_guide_interface_base::base::on_key(_loop, shortcut_sym.raw());
    }

    // Shortcut matching sees the nested substitution (Right Ctrl AS Super, Left
    // Ctrl still Ctrl) — both the modifier bits and the key identity. The pen
    // capture above deliberately took the RAW pair: it records the physical combo
    // the user pressed, not what the compositor binds it to.
    let (shortcut_sym, modifiers) = nested_shortcut_view(_loop, shortcut_sym, modifiers);

    if should_forward::<I>(_loop, keysym, shortcut_sym, key_state, &modifiers) {
        keyboard.input_forward(&mut _loop.state, key_code, key_state, serial, time, mods_changed);
    }
}

/// Inject a synthesized key edge (from the on-screen keyboard) into whatever holds
/// keyboard focus — a focused compositor ICED surface (a settings text field, etc.)
/// OR the focused Wayland client — uniformly. The keycode is run through the live xkb
/// state (`input_intercept`), so it yields the correct keysym/char for the active
/// layout + held modifiers, exactly like a physical key. Deliberately skips the
/// shortcut/overlay routing (`should_forward`): OSK keys are text, not compositor
/// shortcuts. Iced surface focused → dispatch to iced (modifier tracking + char);
/// otherwise → forward to the focused client.
pub fn inject_key(_loop: &mut Loop, key_code: Keycode, pressed: bool, time: u32) {
    let serial = SERIAL_COUNTER.next_serial();
    let key_state = if pressed { KeyState::Pressed } else { KeyState::Released };
    let Some(keyboard) = _loop.state.seat.seat.get_keyboard() else { return };
    let ((keysym, _modifiers), mods_changed) = keyboard.input_intercept(
        &mut _loop.state,
        key_code,
        key_state,
        |_d, modifiers, handle| (handle.modified_sym(), *modifiers),
    );
    let iced_focused = _loop
        .inner
        .surface()
        .registry
        .as_ref()
        .is_some_and(|r| r.keyboard_focus().is_some());
    if iced_focused {
        iced_handle(_loop, keysym, key_state);
    } else {
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
    shortcut_sym: Keysym,
    key_state: KeyState,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> bool {
    // Shortcut matchers (overlay + canvas) get the layout-agnostic `shortcut_sym`; iced
    // TEXT sinks below get `keysym` (the active-layout char being typed).
    if compositor_y5_overlay_interface_keyboard::keyboard::input_received::<I>(
        _loop, shortcut_sym, key_state, modifiers,
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
        compositor_y5_canvas_input_keyboard::keyboard::input_received(key_state, modifiers, shortcut_sym, _loop)
            .is_none();
    if shortcut_intercept {
        return false;
    }

    // wlr exclusive-keyboard grab: we're past every overlay / launcher / iced / shortcut
    // check, so NO compositor modal consumed this key — a mapped Top/Overlay layer surface
    // that requested `Exclusive` owns the keyboard. Focus it (no click required) so this and
    // subsequent keys reach it. `set_focus` no-ops if it's already focused, so this is not a
    // per-key re-switch once it holds focus (windows can't steal it — see press.rs guard).
    exclusive_keyboard_grab(_loop);

    if let Some(result) = wayland_handle(_loop) {
        return !result;
    }
    if let Some(result) = iced_handle(_loop, keysym, key_state) {
        return !result;
    }
    true
}

/// Give keyboard focus to the topmost mapped Top/Overlay layer surface that requested
/// `Exclusive` keyboard interactivity, if it doesn't already hold it. Called only after every
/// compositor overlay/modal declined the key, so those keep the keyboard while active.
fn exclusive_keyboard_grab(_loop: &mut Loop) {
    let Some(excl) = exclusive_layer(_loop) else {
        return;
    };
    let Some(keyboard) = _loop.state.seat.seat.get_keyboard() else {
        return;
    };
    if keyboard.current_focus().as_ref() != Some(&excl) {
        let serial = SERIAL_COUNTER.next_serial();
        keyboard.set_focus(&mut _loop.state, Some(excl), serial);
    }
}

/// The topmost mapped Top/Overlay layer surface with `Exclusive` keyboard interactivity, if
/// any. (Per wlr-layer-shell, exclusive keyboard is only guaranteed for Top/Overlay.)
fn exclusive_layer(_loop: &Loop) -> Option<WlSurface> {
    for output in _loop.inner.space_state().state.outputs() {
        let map = layer_map_for_output(output);
        for band in [Layer::Overlay, Layer::Top] {
            for layer in map.layers_on(band).rev() {
                if layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive {
                    return Some(layer.wl_surface().clone());
                }
            }
        }
    }
    None
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
