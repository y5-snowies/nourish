//! Touch session state machine. Routes every touch event by the session's role,
//! decided at the first finger down:
//!   * finger on a client surface  → forward to `wl_touch` (native multi-touch),
//!   * one finger on the desktop/UI → emulate the pointer (move + tap/drag),
//!   * two+ fingers on the desktop  → compositor gestures (pan/zoom/swipe/fit).
use super::{client, emulate, geom, gesture};
use smithay::backend::input::{Event, InputBackend, TouchEvent};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_gesture_touch::touch::{Mode, Role};

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
        if client::is_client(_loop, world) {
            _loop.inner.touch.role = Role::Client;
            client::down(_loop, id, world, time);
        } else {
            _loop.inner.touch.role = Role::Pointer;
            emulate::move_to(_loop, nx, ny, time);
            // Empty canvas → glide-pan (momentum) on drag; over a window/UI →
            // click/drag. The tap-vs-drag decision is settled at release.
            let glide = !client::over_window(_loop, world);
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
            let world = geom::world(_loop, phys);
            client::down(_loop, id, world, time);
        }
        Role::Pointer => {
            // A second finger on the desktop means the sequence is a gesture, not
            // a tap: release the emulated press and switch to gesture mode.
            emulate::release(_loop, time);
            _loop.inner.touch.role = Role::Gesture;
            let m = mode_for(_loop.inner.touch.len());
            _loop.inner.touch.mode = m;
            gesture::begin(_loop, m, time);
        }
        Role::Gesture => resync(_loop, time),
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
                if _loop.inner.touch.pointer_moved {
                    emulate::pan(_loop, 0.0, 0.0, true); // terminate → launch coast
                } else {
                    // A stationary tap on empty canvas → a click (clear selection).
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
        Role::Idle => {}
    }
    _loop.inner.touch.reset();
}

pub fn frame<I: InputBackend>(_event: &I::TouchFrameEvent, _loop: &mut Loop) {
    if _loop.inner.touch.role == Role::Client {
        client::frame(_loop);
    }
}
