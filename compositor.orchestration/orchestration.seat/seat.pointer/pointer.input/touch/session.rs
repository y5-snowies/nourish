//! Touch session state machine. Routes every touch event by the session's role,
//! decided at the first finger down:
//!   * finger on a client surface  → forward to `wl_touch` (native multi-touch),
//!   * one finger on the desktop/UI → emulate the pointer (move + tap/drag),
//!   * two+ fingers on the desktop  → compositor gestures (pan/zoom/swipe/fit).
use super::{client, edge, emulate, geom, gesture};
use smithay::backend::input::{Event, InputBackend, TouchEvent};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_gesture_touch::touch::{Mode, Role, TouchMode};

/// Gesture mode for a given desktop finger count (0/1 → no gesture).
fn mode_for(n: usize) -> Mode {
    match n {
        2 => Mode::Zoom,
        3 => Mode::Swipe,
        _ if n >= 4 => Mode::Fit,
        _ => Mode::None,
    }
}

/// Re-evaluate the gesture mode after a contact changed (Gesture role only): end
/// the outgoing mode, then begin the incoming one.
fn resync(_loop: &mut Loop, time: u32) {
    let want = mode_for(_loop.inner.touch.len());
    let have = _loop.inner.touch.mode;
    if want == have {
        return;
    }
    gesture::end(_loop, have, time, false);
    _loop.inner.touch.mode = want;
    gesture::begin(_loop, want, time);
}

pub fn down<I: InputBackend>(event: &I::TouchDownEvent, _loop: &mut Loop) {
    let id = i32::from(event.slot());
    let (nx, ny) = emulate::fraction::<I, _>(event);
    let time = event.time_msec();
    let phys = geom::physical(_loop, nx, ny);
    let first = _loop.inner.touch.is_empty();
    _loop.inner.touch.upsert(id, phys);

    if first {
        let world = geom::world(_loop, phys);
        // Compositor iced UI (the touch pane, overview menu, selection bar, …)
        // must take a normal pointer tap in EVERY tool-mode, so it stays usable
        // even in Hand mode (whose canvas taps are inert).
        let over_ui = client::over_iced(_loop, world);
        // `Touch` (default) delegates to a client's `wl_touch` when it bound one;
        // the other tool-modes always emulate the pointer (never forward touch),
        // so the canvas/window sees a mouse and the mode's behaviour applies.
        let forward_client = !over_ui
            && _loop.inner.touch.tool_mode == TouchMode::Touch
            && client::is_client(_loop, world);
        if forward_client {
            _loop.inner.touch.role = Role::Client;
            client::down(_loop, id, world, time);
        } else {
            _loop.inner.touch.role = Role::Pointer;
            emulate::move_to(_loop, nx, ny, time);
            // Glide-pan (momentum) vs press-on-down. Compositor UI always presses
            // (so its buttons work in every mode). Otherwise `Pointer`/`Select`/
            // `Hand` press-on-down — Hand's press drives the armed canvas Hand
            // grab (1:1 pan, ignores windows), Pointer/Select drive the pointer /
            // Select tool. Only `Touch` mode glides, and only off a window/UI.
            let glide = if over_ui {
                false
            } else {
                match _loop.inner.touch.tool_mode {
                    TouchMode::Pointer | TouchMode::Select | TouchMode::Hand => false,
                    TouchMode::Touch => !client::over_window(_loop, world),
                }
            };
            _loop.inner.touch.pointer_glide = glide;
            _loop.inner.touch.pointer_moved = false;
            _loop.inner.touch.prev_centroid = _loop.inner.touch.centroid();
            if !glide {
                emulate::press(_loop, time);
            }
        }
        return;
    }

    match _loop.inner.touch.role {
        Role::Client => {
            // A 2-finger swipe from a screen edge is swallowed even over a client:
            // cancel the `wl_touch` stream and claim it as an edge gesture.
            if _loop.inner.touch.len() == 2 && edge::maybe_begin(_loop, time) {
                client::cancel(_loop);
            } else {
                let world = geom::world(_loop, phys);
                client::down(_loop, id, world, time);
            }
        }
        Role::Pointer => {
            // Second finger: end the emulated press. A 2-finger swipe from a screen
            // edge is swallowed as an edge gesture; otherwise it's a camera gesture.
            emulate::release(_loop, time);
            if !edge::maybe_begin(_loop, time) {
                _loop.inner.touch.role = Role::Gesture;
                let m = mode_for(_loop.inner.touch.len());
                _loop.inner.touch.mode = m;
                gesture::begin(_loop, m, time);
            }
        }
        Role::Gesture => resync(_loop, time),
        Role::Edge => {}
        Role::Idle => {}
    }
}

pub fn motion<I: InputBackend>(event: &I::TouchMotionEvent, _loop: &mut Loop) {
    let id = i32::from(event.slot());
    if _loop.inner.touch.contacts.iter().all(|c| c.slot != id) {
        return; // motion for a finger we aren't tracking
    }
    let (nx, ny) = emulate::fraction::<I, _>(event);
    let time = event.time_msec();
    let phys = geom::physical(_loop, nx, ny);
    _loop.inner.touch.upsert(id, phys);
    match _loop.inner.touch.role {
        Role::Client => {
            let world = geom::world(_loop, phys);
            client::motion(_loop, id, world, time);
        }
        Role::Pointer => {
            if _loop.inner.touch.pointer_glide {
                // Glide-pan the canvas by the finger delta (momentum).
                _loop.inner.touch.pointer_moved = true;
                let c = _loop.inner.touch.centroid();
                let prev = _loop.inner.touch.prev_centroid;
                _loop.inner.touch.prev_centroid = c;
                emulate::pan(_loop, c.x - prev.x, c.y - prev.y, true);
            } else {
                emulate::move_to(_loop, nx, ny, time);
            }
        }
        Role::Gesture => gesture::update(_loop, time),
        Role::Edge => edge::update(_loop, time),
        Role::Idle => {}
    }
}

pub fn up<I: InputBackend>(event: &I::TouchUpEvent, _loop: &mut Loop) {
    let id = i32::from(event.slot());
    let time = event.time_msec();
    if !_loop.inner.touch.remove(id) {
        return;
    }
    let role = _loop.inner.touch.role;
    let empty = _loop.inner.touch.is_empty();
    match role {
        Role::Client => client::up(_loop, id, time),
        Role::Pointer => {
            if _loop.inner.touch.pointer_glide {
                // Glide is Touch-mode only (empty canvas): fling on a drag, or a
                // stationary tap → a click that clears the selection.
                if _loop.inner.touch.pointer_moved {
                    emulate::pan(_loop, 0.0, 0.0, true); // terminate → launch coast
                } else {
                    emulate::press(_loop, time);
                    emulate::release(_loop, time);
                }
            } else {
                emulate::release(_loop, time);
            }
        }
        Role::Gesture => {
            if empty {
                let m = _loop.inner.touch.mode;
                gesture::end(_loop, m, time, false);
            } else {
                resync(_loop, time);
            }
        }
        Role::Edge => {}
        Role::Idle => {}
    }
    if empty {
        _loop.inner.touch.reset();
    }
}

pub fn cancel<I: InputBackend>(_event: &I::TouchCancelEvent, _loop: &mut Loop) {
    match _loop.inner.touch.role {
        Role::Client => client::cancel(_loop),
        Role::Pointer => emulate::release(_loop, 0),
        Role::Gesture => {
            let m = _loop.inner.touch.mode;
            gesture::end(_loop, m, 0, true);
        }
        Role::Edge => {}
        Role::Idle => {}
    }
    _loop.inner.touch.reset();
}

pub fn frame<I: InputBackend>(_event: &I::TouchFrameEvent, _loop: &mut Loop) {
    if _loop.inner.touch.role == Role::Client {
        client::frame(_loop);
    }
}
