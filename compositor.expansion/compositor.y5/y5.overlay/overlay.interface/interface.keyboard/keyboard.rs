use smithay::backend::input::{InputBackend, KeyState};
use smithay::backend::session::Session;
use smithay::input::keyboard::{Keysym, ModifiersState};
use compositor_support_library_input_keyboard_base::keyboard::combo::KeyCombo;
use compositor_support_library_input_keyboard_base::keyboard::handler::ShortcutHandler;
use compositor_support_library_input_keyboard_base::keyboard::key::Key;
use compositor_support_library_input_keyboard_base::shortcut;
use compositor_support_library_input_keyboard_format::format;
use compositor_developer_environment_keybinding_base::base::{KeyBindings, KeyRow};

use compositor_orchestration_core_state_base::Loop;

pub fn input_received<I: InputBackend>(
    state: &mut Loop,
    keysym: Keysym,
    key_state: KeyState,
    modifiers: &ModifiersState,
) -> bool {
    let key = Key::from_keysym(keysym);
    if key.is_none() {
        return false;
    }

    let is_press = key_state == KeyState::Pressed;
    // Only handle presses.
    if !is_press {
        return false;
    }

    // Build the handler vector, applying the user's keybinding.json overrides.
    let handlers: Vec<ShortcutHandler<Loop>> =
        inline_shortcut_handlers(state.inner.storage.nested, &state.inner.keybinding);

    for handler in handlers {
        if handler.combo.matches(modifiers, key) {
            if (handler.action)(state) {
                return true;
            }
        }
    }

    // Add Playback device control, TTY switches.
    return false;
}

const VOLUME_STEP: f64 = 0.05;

/// One bindable shortcut: a stable id (referenced by keybinding.json + the
/// settings Keys tab), a human label, the built-in default combo, and the action.
struct Bind {
    id: &'static str,
    label: &'static str,
    default: KeyCombo,
    action: Box<dyn Fn(&mut Loop) -> bool>,
}

/// The single source of truth for every compositor shortcut. `inline_shortcut_handlers`
/// turns these into live handlers (with overrides applied); `registry` exposes them
/// to the settings Keys tab.
fn bindings() -> Vec<Bind> {
    vec![
        Bind { id: "sleep", label: "Sleep", default: shortcut!(Super + Alt + L), action: Box::new(|s| { sleep(s); true }) },
        Bind { id: "world_picker", label: "Open world picker", default: shortcut!(Super + K), action: Box::new(|s| { compositor_y5_picker_interface_entry::entry::toggle(s); true }) },
        // (Settings has no global shortcut — reachable only via the overview Settings tab.)
        // Removed (per request): world-switch test shortcuts, Escape/cancel-picker,
        // VT switches, and all sink/media shortcuts — deactivated AND not listed.
    ]
}

/// Build the live handlers, applying keybinding.json overrides (parse-or-default)
/// and the nested-mode Super→Ctrl remap.
pub fn inline_shortcut_handlers(nosuper: bool, overrides: &KeyBindings) -> Vec<ShortcutHandler<Loop>> {
    let mut handlers: Vec<ShortcutHandler<Loop>> = bindings()
        .into_iter()
        .filter_map(|b| match overrides.combo_for(b.id) {
            // Empty override string = explicitly disabled: no handler at all.
            Some("") => None,
            Some(s) => Some(ShortcutHandler { combo: format::parse_combo(s).unwrap_or(b.default), action: b.action }),
            None => Some(ShortcutHandler { combo: b.default, action: b.action }),
        })
        .collect();

    if nosuper {
        for w in &mut handlers {
            if w.combo.modifiers.logo {
                w.combo.modifiers.logo = false;
                w.combo.modifiers.ctrl = true;
            }
        }
    }

    // Always-on, non-configurable system handlers (cannot be rebound or disabled
    // — they are the escape hatches). Appended after the configurable bindings.
    handlers.extend(fixed_handlers());
    handlers
}

/// Critical, non-rebindable handlers: TTY/VT switches (the compositor holds the
/// VT in graphics mode, so without these Ctrl+Alt+F-keys do nothing) and the
/// Escape-cancels-picker fall-through.
fn fixed_handlers() -> Vec<ShortcutHandler<Loop>> {
    fn vt(n: u32) -> Box<dyn Fn(&mut Loop) -> bool> {
        Box::new(move |s| {
            tty(s, n);
            true
        })
    }
    vec![
        ShortcutHandler { combo: shortcut!(Ctrl + Alt + SwitchVt1), action: vt(1) },
        ShortcutHandler { combo: shortcut!(Ctrl + Alt + SwitchVt2), action: vt(2) },
        ShortcutHandler { combo: shortcut!(Ctrl + Alt + SwitchVt3), action: vt(3) },
        ShortcutHandler { combo: shortcut!(Ctrl + Alt + SwitchVt4), action: vt(4) },
        ShortcutHandler { combo: shortcut!(Ctrl + Alt + SwitchVt5), action: vt(5) },
        ShortcutHandler { combo: shortcut!(Ctrl + Alt + SwitchVt6), action: vt(6) },
        ShortcutHandler {
            combo: shortcut!(Escape),
            action: Box::new(|s| {
                if s.inner.worlds.active_id() == compositor_y5_picker_system_base::base::PICKER_WORLD {
                    compositor_y5_picker_interface_base::base::cancel(s);
                    true
                } else {
                    false
                }
            }),
        },
        // Hardware media/volume keys — functional but not user-configurable.
        ShortcutHandler { combo: shortcut!(AudioRaiseVolume), action: Box::new(|s| {
            if let Some(a) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::AUDIO_MUT) { let _ = a.adjust_volume(VOLUME_STEP); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioLowerVolume), action: Box::new(|s| {
            if let Some(a) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::AUDIO_MUT) { let _ = a.adjust_volume(-VOLUME_STEP); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioMute), action: Box::new(|s| {
            if let Some(a) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::AUDIO_MUT) { let _ = a.toggle_mute(); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioPlay), action: Box::new(|s| {
            if let Some(m) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::MEDIA_MUT) { let _ = m.play_pause(); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioPause), action: Box::new(|s| {
            if let Some(m) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::MEDIA_MUT) { let _ = m.pause(); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioStop), action: Box::new(|s| {
            if let Some(m) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::MEDIA_MUT) { let _ = m.stop(); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioNext), action: Box::new(|s| {
            if let Some(m) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::MEDIA_MUT) { let _ = m.next(); }
            true
        }) },
        ShortcutHandler { combo: shortcut!(AudioPrev), action: Box::new(|s| {
            if let Some(m) = s.inner.kernel.get_mut(&compositor_orchestration_driver_audio_base::base::MEDIA_MUT) { let _ = m.previous(); }
            true
        }) },
    ]
}

/// Read-only rows for the always-on system handlers (shown under "Built-in").
pub fn fixed() -> Vec<KeyRow> {
    let mk = |label: &str, combo: &str| KeyRow {
        id: String::new(),
        label: label.to_string(),
        default: combo.to_string(),
        combo: combo.to_string(),
        editable: false,
    };
    vec![
        mk("Switch to VT 1", "Ctrl+Alt+F1"),
        mk("Switch to VT 2", "Ctrl+Alt+F2"),
        mk("Switch to VT 3", "Ctrl+Alt+F3"),
        mk("Switch to VT 4", "Ctrl+Alt+F4"),
        mk("Switch to VT 5", "Ctrl+Alt+F5"),
        mk("Switch to VT 6", "Ctrl+Alt+F6"),
        mk("Cancel picker", "Esc"),
        mk("Volume up", "VolumeUp key"),
        mk("Volume down", "VolumeDown key"),
        mk("Mute", "Mute key"),
        mk("Play / Pause", "Play key"),
        mk("Pause media", "Pause key"),
        mk("Stop media", "Stop key"),
        mk("Next track", "Next key"),
        mk("Previous track", "Prev key"),
    ]
}

/// All shortcuts as `(id, label, default, effective)` rows for the settings Keys tab.
pub fn registry(overrides: &KeyBindings) -> Vec<KeyRow> {
    bindings()
        .into_iter()
        .map(|b| {
            let default = format::combo_string(&b.default);
            let combo = overrides.combo_for(b.id).map(str::to_string).unwrap_or_else(|| default.clone());
            KeyRow { id: b.id.to_string(), label: b.label.to_string(), default, combo, editable: true }
        })
        .collect()
}

fn tty(state: &mut Loop, num: u32) {
    error!("VT switch to {:?}", num);
    state.loop_handle.insert_idle(move |state| {
        // Do not perform the action again when paused.
        if let compositor_orchestration_core_state_base::state::StatusSession::Paused =
            state.inner.status_session
        {
            error!("VT already paused");
            return;
        }

        let Some(_tty) = &mut state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).tty else {
            error!("VT unavailable");
            return;
        };

        if let Some(ses) = &mut state.state.seat.libseat {
            ses.change_vt(num as i32);
        }
    });
}

fn sleep(state: &mut Loop) {
    error!("Sleep");
    if matches!(state.inner.status, compositor_orchestration_core_state_base::state::Status::Locked { .. }) {
        return; // already locked
    }
    // Status set SYNCHRONOUSLY (sleep lock) + flag the engage, then wake the
    // control-plane ping: its source drains `lock_engage` and runs the renderer-free
    // `lock_logical` off-frame, event-driven (not polled per frame).
    state.inner.status = compositor_orchestration_core_state_base::state::Status::Locked {
        pending: true,
        sleep: true,
        time: std::time::Instant::now(),
    };
    state.inner.lock_engage = true;
    state.inner.ping_control();
}

/// TEMPORARY (sanity test): make test-world `slot` (0=main, 1/2=pre-created
/// spatial worlds) the active + spawn-target world, exercising world delegation.
fn switch_world(state: &mut Loop, slot: usize) -> bool {
    let target = state.inner.kernel.get(&compositor_orchestration_core_state_base::state::TEST_WORLDS)[slot];
    // Capture all outputs + positions before switch (multi-output: map every one
    // into the target world's fresh Space at its real position, not just the first).
    let outputs: Vec<(smithay::output::Output, smithay::utils::Point<i32, smithay::utils::Logical>)> =
        state.inner.space_state().state.outputs().map(|o| {
            let loc = state.inner.space_state().state.output_geometry(o).map(|g| g.loc).unwrap_or_default();
            (o.clone(), loc)
        }).collect();

    state.inner.worlds.switch(target, &state.inner.kernel);
    state.inner.worlds.set_spawn_target(target);
    info!("world switch -> slot {slot} (world {target})");

    if state.inner.space_state().state.outputs().next().is_none() {
        for (output, loc) in &outputs {
            state.inner.space_state_mut().state.map_output(output, *loc);
        }
    }
    true
}
