use smithay::backend::input::KeyState;
use smithay::desktop::Window;
use smithay::input::keyboard::{KeysymHandle, ModifiersState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Size};
use uuid::Uuid;
use compositor_support_action_camera_find_base::find::Direction;
use compositor_support_action_camera_fit_aspect::aspect;
use compositor_y5_camera_zone_state::state::{Zone, ZoneSpecifier};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::Status;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_navigator_interface_base::interface::move_direction;
use compositor_y5_navigator_travel_state::state::{Target, Travel};
use compositor_support_library_input_keyboard_base::keyboard::combo::KeyCombo;
use compositor_support_library_input_keyboard_base::keyboard::handler::ShortcutHandler;
use compositor_support_library_input_keyboard_base::keyboard::key::Key;
use compositor_support_library_input_keyboard_base::shortcut;
use compositor_support_library_input_keyboard_format::format;
use compositor_developer_environment_keybinding_base::base::{KeyBindings, KeyRow};
use compositor_y5_window_interface_draw::fullscreen::fullscreen_unset_focused;
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_y5_window_lifecycle_interface::interface::TransformUpdate;

pub fn input_received(
    key_state: KeyState,
    modifiers: &ModifiersState,
    state: &mut Loop,
    key: Option<Key>,
) -> Option<bool> {
    if key.is_none() {
        return Some(true);
    }

    let is_press = key_state == KeyState::Pressed;
    // Only handle presses.
    if !is_press {
        return Some(true);
    }

    // Build the vector manually using standard Rust struct initialization.
    // We only use the single `shortcut!` macro to generate the KeyCombo.
    let handlers: Vec<ShortcutHandler<Loop>> =
        inline_shortcut_handlers(state.inner.storage.nested, &state.inner.keybinding);

    // Iterate through the vector (Top to Bottom priority)
    for handler in handlers {
        if handler.combo.matches(modifiers, key) {
            if (handler.action)(state) {
                return None;
            }
        }
    }

    Some(true)
}

fn launcher_delegate(p0: &mut Loop) {
    compositor_y5_launcher_interface_base::interface::start_defered(p0)
}

fn zone_delegate(state: &mut Loop, zone: &str, register: bool) {
    if register {
        // CHECK: From given selection, it must be availabvle windows only. doesnt matter since they wont participate next step.
        // they may become "hidden". this needs to be handled on window close ( whenever space deletes that window )
        let selected: Vec<Uuid> = state.inner.select()
            
            .Selection
            .clone()
            .iter()
            .filter_map(|w| w.uuid())
            .collect();

        if selected.is_empty() {
            let position = state.inner.camera_mut().transform.position();
            let position = (position.x, position.y);
            let zoom = *state.inner.camera_mut().transform.zoom();

            state.inner.camera_mut().zone.zone.insert(
                zone.into(),
                Zone {
                    specifier: ZoneSpecifier::Camera {
                        zoom: zoom,
                        position: position,
                    },
                },
            );
        } else {
            state.inner.camera_mut().zone.zone.insert(
                zone.into(),
                Zone {
                    specifier: ZoneSpecifier::Element { UUID: selected },
                },
            );
        }
    } else {
        let zone = String::from(zone);
        let zone = state.inner.camera().zone.zone.get(&zone);
        if zone.is_none() {
            return;
        }
        let zone = zone.unwrap();

        match &zone.specifier {
            ZoneSpecifier::Element { UUID } => {
                let windows: Vec<Window> = state
                    .inner.space_state()
                    .state
                    .elements()
                    .filter_map(|f| {
                        let f = f.clone();
                        let uuid = f.uuid();
                        if uuid.is_none() {
                            return None;
                        }
                        let uuid = uuid.unwrap();

                        if !UUID.contains(&uuid) {
                            return None;
                        }

                        return Some(f);
                    })
                    .collect();

                let windows = windows.iter().map(|w| w).collect();
                let result =
                    compositor_y5_navigator_travel_machine::view::view(state, windows, false);
                let zoom = result.zoom.and_then(|target| {
                    return Some(Target {
                        start: None,
                        target,
                    });
                });

                let position = result.position.and_then(|target| {
                    return Some(Target {
                        start: None,
                        target: result.position.unwrap(),
                    });
                });

                let travel = Travel {
                    position: position,
                    zoom: zoom,
                    duration: None,
                    time_start: None,
                };

                state.inner.navigator_mut().set(
                    compositor_y5_navigator_state_base::state::State::Travel(travel),
                );
            }
            ZoneSpecifier::Camera { position, zoom } => {
                let zoom = Some(Target {
                    start: None,
                    target: zoom.clone(),
                });

                let position = Some(Target {
                    start: None,
                    target: position.clone(),
                });

                let travel = Travel {
                    position: position,
                    zoom: zoom,
                    duration: None,
                    time_start: None,
                };

                state.inner.navigator_mut().set(
                    compositor_y5_navigator_state_base::state::State::Travel(travel),
                );
            }
        }
    }
}

fn group_delegate(state: &mut Loop, join: bool,) {
    if join {
        compositor_y5_group_interface_base::interface::selection_set_join(state);
        
    } else {
    
        compositor_y5_group_interface_base::interface::selection_set(state);
    }
}

fn zoom_delegate(state: &mut Loop, zoom_1: bool, fit_1: bool) {
    compositor_y5_navigator_interface_base::interface::fit(state, zoom_1, fit_1);
}

// // // Only locks. results in showing GDM
// fn lock(s: &mut Loop) {
//     // if s.is_locked() {
//     //     return;
//     // }
//
//     // This is what actually locks the screen. The client connects to your
//     // ext-session-lock-v1 implementation, your SessionLockHandler::lock fires,
//     // you blank outputs, present a frame, call SessionLocker::lock(), done.
//     let _ = Command::new("gtklock") // or "swaylock", "hyprlock"
//         .spawn();
//
//     // Tell the rest of the system we're locked, for tools like
//     // gnome-keyring that watch this flag. Optional but polite.
//     let _ = Command::new("loginctl")
//         .args(["lock-session"])
//         .status();
//
//     // // Avoid double-locking.
//     // if s.is_locked() {
//     //     return;
//     // }
//     //
//     //
//     //
//     // // Tell logind the session is logically locked. lock clients and
//     // // other listeners (e.g. gnome-keyring) observe this via PropertiesChanged.
//     // if let Ok(conn) = Connection::system() {
//     //     let _ = conn.call_method(
//     //         Some("org.freedesktop.login1"),
//     //         "/org/freedesktop/login1/session/auto",
//     //         Some("org.freedesktop.login1.Session"),
//     //         "SetLockedHint",
//     //         &(true,),
//     //     );
//     // }
//     //
//     //
//     // let _ = Command::new("loginctl")
//     //     .args(["lock-session"]) // operates on the caller's session by default
//     //     .status();
//     //
//     // Spawn whichever ext-session-lock-v1 client you want as the unlock UI.
//     // // gtklock / hyprlock / swaylock all work. Configure this; don't hardcode.
//     // let _ = Command::new(&s.config.lock_command) // e.g. "gtklock"
//     //     .spawn();
//
//     // From here on, your SessionLockHandler::lock() impl will fire when
//     // the client binds ext_session_lock_manager_v1 and calls lock(). That
//     // handler is where you call SessionLocker::lock() after presenting a
//     // blanked frame on every output.
//
// }
//
// // fn lock(s: &mut Loop) {
// //     if s.is_locked() {
// //         return;
// //     }
// //
// //     // This is what actually locks the screen. The client connects to your
// //     // ext-session-lock-v1 implementation, your SessionLockHandler::lock fires,
// //     // you blank outputs, present a frame, call SessionLocker::lock(), done.
// //     let _ = Command::new("gtklock") // or "swaylock", "hyprlock"
// //         .spawn();
// //
// //     // Tell the rest of the system we're locked, for tools like
// //     // gnome-keyring that watch this flag. Optional but polite.
// //     let _ = Command::new("loginctl")
// //         .args(["lock-session"])
// //         .status();
// // }
//
// //
//
//
// // // Executes both lock and sleep.
// fn sleep(s: &mut Loop) {
//     // There is an "inhibitor" way to way for lock which is better.
//     lock(s);
//     // s.wait_until_locked(Duration::from_millis(500));
//
//     let _ = Command::new("systemctl")
//         .arg("suspend")
//         .spawn(); // fire and forget; suspend doesn't return until resume
// }
//
// // Completely terminate the session and all process within it.
// // It needs to be like regular gnome logout.
fn lock(s: &mut Loop) {
    if matches!(s.inner.status, Status::Locked { .. }) {
        return; // already locked
    }
    // Set the lock status SYNCHRONOUSLY (so it holds even with no output) + flag the
    // engage, then wake the control-plane ping: its source drains `lock_engage` and
    // runs the renderer-free `lock_logical` off-frame, event-driven (not polled).
    // (This crate can't call `lock_interface` directly — `lock_interface →
    // seat_keyboard_input → this`.)
    s.inner.status = Status::Locked { pending: true, sleep: false, time: std::time::Instant::now() };
    s.inner.lock_engage = true;
    s.inner.ping_control();
}

struct Bind {
    id: &'static str,
    label: &'static str,
    default: KeyCombo,
    action: Box<dyn Fn(&mut Loop) -> bool>,
}

/// Single source of truth for the canvas/navigation/zone/lock shortcuts.
fn bindings() -> Vec<Bind> {
    vec![
        Bind { id: "launcher", label: "Open launcher", default: shortcut!(Super + N), action: Box::new(|s| { launcher_delegate(s); true }) },
        Bind { id: "capture", label: "Screen capture", default: shortcut!(Super + S), action: Box::new(|s| { compositor_y5_graphic_capture_interface::interface::request_setup(s); true }) },
        Bind { id: "fullscreen_exit", label: "Exit fullscreen", default: shortcut!(F11), action: Box::new(|s| fullscreen_unset_focused(s)) },
        Bind { id: "zone_1", label: "Zone 1", default: shortcut!(Super + Num1), action: Box::new(|s| { zone_delegate(s, "f1", false); true }) },
        Bind { id: "zone_2", label: "Zone 2", default: shortcut!(Super + Num2), action: Box::new(|s| { zone_delegate(s, "f2", false); true }) },
        Bind { id: "zone_3", label: "Zone 3", default: shortcut!(Super + Num3), action: Box::new(|s| { zone_delegate(s, "f3", false); true }) },
        Bind { id: "zone_4", label: "Zone 4", default: shortcut!(Super + Num4), action: Box::new(|s| { zone_delegate(s, "f4", false); true }) },
        Bind { id: "zone_5", label: "Zone 5", default: shortcut!(Super + Num5), action: Box::new(|s| { zone_delegate(s, "f5", false); true }) },
        Bind { id: "zone_6", label: "Zone 6", default: shortcut!(Super + Num6), action: Box::new(|s| { zone_delegate(s, "f6", false); true }) },
        Bind { id: "zone_set_1", label: "Set zone 1", default: shortcut!(Super + Shift + Num1), action: Box::new(|s| { zone_delegate(s, "f1", true); true }) },
        Bind { id: "zone_set_2", label: "Set zone 2", default: shortcut!(Super + Shift + Num2), action: Box::new(|s| { zone_delegate(s, "f2", true); true }) },
        Bind { id: "zone_set_3", label: "Set zone 3", default: shortcut!(Super + Shift + Num3), action: Box::new(|s| { zone_delegate(s, "f3", true); true }) },
        Bind { id: "zone_set_4", label: "Set zone 4", default: shortcut!(Super + Shift + Num4), action: Box::new(|s| { zone_delegate(s, "f4", true); true }) },
        Bind { id: "zone_set_5", label: "Set zone 5", default: shortcut!(Super + Shift + Num5), action: Box::new(|s| { zone_delegate(s, "f5", true); true }) },
        Bind { id: "zone_set_6", label: "Set zone 6", default: shortcut!(Super + Shift + Num6), action: Box::new(|s| { zone_delegate(s, "f6", true); true }) },
        Bind { id: "group_all", label: "Group (join)", default: shortcut!(Super + Alt + G), action: Box::new(|s| { group_delegate(s, true); true }) },
        Bind { id: "group", label: "Group", default: shortcut!(Super + G), action: Box::new(|s| { group_delegate(s, false); true }) },
        Bind { id: "zoom_fit", label: "Zoom: fit", default: shortcut!(Super + Alt + F), action: Box::new(|s| { zoom_delegate(s, false, false); true }) },
        Bind { id: "zoom_focus", label: "Zoom: focus", default: shortcut!(Super + F), action: Box::new(|s| { zoom_delegate(s, true, false); true }) },
        Bind { id: "zoom_focus_max", label: "Fit to screen", default: shortcut!(Super + Shift + Alt + F), action: Box::new(|s| { zoom_delegate(s, true, true); true }) },
        Bind { id: "nav_right", label: "Navigate right", default: shortcut!(Super + Right), action: Box::new(|s| { move_direction(s, Direction::Right, true); true }) },
        Bind { id: "nav_left", label: "Navigate left", default: shortcut!(Super + Left), action: Box::new(|s| { move_direction(s, Direction::Left, true); true }) },
        Bind { id: "nav_up", label: "Navigate up", default: shortcut!(Super + Up), action: Box::new(|s| { move_direction(s, Direction::Up, true); true }) },
        Bind { id: "nav_down", label: "Navigate down", default: shortcut!(Super + Down), action: Box::new(|s| { move_direction(s, Direction::Down, true); true }) },
        Bind { id: "move_right", label: "Navigate right and zoom", default: shortcut!(Super + Alt + Right), action: Box::new(|s| { move_direction(s, Direction::Right, false); true }) },
        Bind { id: "move_left", label: "Navigate left and zoom", default: shortcut!(Super + Alt + Left), action: Box::new(|s| { move_direction(s, Direction::Left, false); true }) },
        Bind { id: "move_up", label: "Navigate up and zoom", default: shortcut!(Super + Alt + Up), action: Box::new(|s| { move_direction(s, Direction::Up, false); true }) },
        Bind { id: "move_down", label: "Navigate down and zoom", default: shortcut!(Super + Alt + Down), action: Box::new(|s| { move_direction(s, Direction::Down, false); true }) },
        Bind { id: "lock", label: "Lock screen", default: shortcut!(Super + L), action: Box::new(|s| { lock(s); true }) },
    ]
}

/// Build the live handlers with keybinding.json overrides (parse-or-default;
/// empty override = disabled) and the nested-mode Super→Ctrl remap.
pub fn inline_shortcut_handlers(nosuper: bool, overrides: &KeyBindings) -> Vec<ShortcutHandler<Loop>> {
    let mut handlers: Vec<ShortcutHandler<Loop>> = bindings()
        .into_iter()
        .filter_map(|b| match overrides.combo_for(b.id) {
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
    handlers
}

/// All canvas shortcuts as Keys-tab rows (id, label, default, effective combo).
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

/// Built-in, NON-rebindable shortcuts surfaced read-only in the Keys tab: the
/// Super-held canvas grab tools (modifier-only combos in `input::input_received`).
pub fn fixed() -> Vec<KeyRow> {
    let mk = |label: &str, logo: bool, ctrl: bool, alt: bool, shift: bool| {
        let c = KeyCombo {
            modifiers: ModifiersState { logo, ctrl, alt, shift, ..ModifiersState::default() },
            key: None,
        };
        let s = format::combo_string(&c);
        KeyRow { id: String::new(), label: label.to_string(), default: s.clone(), combo: s, editable: false }
    };
    vec![
        mk("Move / pan window (hold + drag)", true, false, false, false),
        mk("Scale window (hold + drag)", true, false, false, true),
        mk("Select box (hold + drag)", true, false, true, false),
        mk("Select box, add (hold + drag)", true, false, true, true),
        mk("Hand tool", true, true, true, false),
    ]
}
