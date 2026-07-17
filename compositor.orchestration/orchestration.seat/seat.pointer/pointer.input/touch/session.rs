//! Touch session state machine. Routes every touch event by the session's role,
//! decided at the first finger down:
//!   * finger on a client surface  → forward to `wl_touch` (native multi-touch),
//!   * one finger on the desktop/UI → emulate the pointer (move + tap/drag),
//!   * two+ fingers on the desktop  → compositor gestures (pan/zoom/swipe/fit).
use super::{client, edge, emulate, geom, gesture};
use smithay::backend::input::{Event, InputBackend, TouchEvent};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{CanvasGrab, TargetOption};
use compositor_orchestration_seat_gesture_touch::touch::{Mode, Role, TouchMode};

/// Max lifetime of a pointer-mode 2-finger tap that still counts as a right click
/// (a slower two-finger hold is not a tap).
const RTAP_MAX_MS: u32 = 300;

/// Gesture mode for a given desktop finger count (0/1 → no gesture).
fn mode_for(n: usize) -> Mode {
    match n {
        2 => Mode::Zoom,
        3 => Mode::Swipe,
        _ if n >= 4 => Mode::Fit,
        _ => Mode::None,
    }
}

/// Disarm the transient Select-tool grab armed for a touch sequence, so the mouse's
/// canvas grab returns to normal between touches. No-op when no touch Select grab set.
fn disarm_select(_loop: &mut Loop) {
    if matches!(
        _loop.inner.canvas_mut().Grab,
        CanvasGrab::Target(TargetOption::Select { .. })
    ) {
        _loop.inner.canvas_mut().Grab = CanvasGrab::None;
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
        // Touch tool-modes are TOUCH-EXCLUSIVE: arm the canvas Select tool only for
        // the life of THIS touch sequence (disarmed on lift), so the mouse — which
        // shares the canvas grab — is never switched into Select by the touch pane.
        if _loop.inner.touch.tool_mode == TouchMode::Select {
            _loop.inner.canvas_mut().Grab =
                CanvasGrab::Target(TargetOption::Select { Append: true });
        }
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
            let glide = if _loop.inner.touch.tool_mode == TouchMode::Hand {
                // Hand pans the world under a single finger ANYWHERE (windows,
                // placeholders, empty canvas) via glide — NO persistent canvas grab,
                // so the touch pane stays tappable to switch modes. Only SCREEN-space
                // compositor UI (the pane / a docked toolbar) takes a normal tap.
                !client::over_screen_iced(_loop, world)
            } else if over_ui {
                false
            } else {
                match _loop.inner.touch.tool_mode {
                    TouchMode::Pointer | TouchMode::Select => false,
                    TouchMode::Hand => unreachable!(),
                    TouchMode::Touch => !client::over_window(_loop, world),
                }
            };
            _loop.inner.touch.pointer_glide = glide;
            _loop.inner.touch.pointer_moved = false;
            _loop.inner.touch.prev_centroid = _loop.inner.touch.centroid();
            // Pointer mode over a surface DEFERS the left press: it's held back until
            // the touch proves to be a click/drag (motion or lift) rather than the
            // start of a 2-finger gesture, so a 2-finger tap fires a clean right click
            // with no stray left click. UI taps and the other press-on-down modes
            // (Select/Hand) are unaffected — they still press immediately.
            let defer = _loop.inner.touch.tool_mode == TouchMode::Pointer && !over_ui;
            _loop.inner.touch.pending_press = !glide && defer;
            if !glide && !defer {
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
            // Second finger: end the primary press. If it was deferred (pointer mode)
            // it never went out — just drop it, so the 2-finger gesture leaves no
            // stray left click. Otherwise release the held button. A 2-finger swipe
            // from a screen edge is swallowed as an edge gesture; else a camera gesture.
            if _loop.inner.touch.pending_press {
                _loop.inner.touch.pending_press = false;
            } else if !_loop.inner.touch.pointer_glide {
                // A glide (Hand, or Touch over empty canvas) never pressed a button, so
                // there is nothing to release; only a real press-on-down does.
                emulate::release(_loop, time);
            }
            if !edge::maybe_begin(_loop, time) {
                _loop.inner.touch.role = Role::Gesture;
                let m = mode_for(_loop.inner.touch.len());
                _loop.inner.touch.mode = m;
                gesture::begin(_loop, m, time);
                // Pointer mode: a 2-finger tap (quick, still) is a right click — arm
                // the candidate now; `gesture::update` / a 3rd finger disqualify it.
                if _loop.inner.touch.tool_mode == TouchMode::Pointer && _loop.inner.touch.len() == 2 {
                    _loop.inner.touch.rtap_candidate = true;
                    _loop.inner.touch.rtap_ms = time;
                    _loop.inner.touch.rtap_origin = _loop.inner.touch.centroid();
                }
            }
        }
        Role::Gesture => {
            // A third finger rules out the 2-finger-tap right click.
            if _loop.inner.touch.len() > 2 {
                _loop.inner.touch.rtap_candidate = false;
            }
            resync(_loop, time);
        }
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
                // Pointer mode: the first motion turns a deferred tap into a drag —
                // send the held-back press now so the button stays down through it.
                if _loop.inner.touch.pending_press {
                    _loop.inner.touch.pending_press = false;
                    emulate::press(_loop, time);
                }
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
                // A drag flings/coasts; a stationary tap in Touch mode is a click
                // (clears the selection), but in Hand mode it is a NO-OP (the Hand
                // tool never clicks the canvas/window under the finger).
                if _loop.inner.touch.pointer_moved {
                    emulate::pan(_loop, 0.0, 0.0, true); // terminate → launch coast
                } else if _loop.inner.touch.tool_mode != TouchMode::Hand {
                    emulate::press(_loop, time);
                    emulate::release(_loop, time);
                }
            } else if _loop.inner.touch.pending_press {
                // Stationary pointer-mode tap: the press was deferred, so synthesize
                // the whole click now (press + release) at the tap location.
                _loop.inner.touch.pending_press = false;
                emulate::press(_loop, time);
                emulate::release(_loop, time);
            } else {
                emulate::release(_loop, time);
            }
        }
        Role::Gesture => {
            if empty {
                let m = _loop.inner.touch.mode;
                gesture::end(_loop, m, time, false);
                // A quick, still 2-finger tap (pointer mode) → right click at its
                // centroid. `rtap_candidate` survives only if nothing panned/pinched
                // and no 3rd finger arrived; the time gate rejects a slow hold.
                if _loop.inner.touch.rtap_candidate
                    && time.wrapping_sub(_loop.inner.touch.rtap_ms) <= RTAP_MAX_MS
                {
                    let origin = _loop.inner.touch.rtap_origin;
                    let (nx, ny) = geom::fraction_of(_loop, origin);
                    emulate::move_to(_loop, nx, ny, time);
                    emulate::right_click(_loop, time);
                }
            } else {
                resync(_loop, time);
            }
        }
        Role::Edge => {}
        Role::Idle => {}
    }
    if empty {
        disarm_select(_loop);
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
    disarm_select(_loop);
    _loop.inner.touch.reset();
}

pub fn frame<I: InputBackend>(_event: &I::TouchFrameEvent, _loop: &mut Loop) {
    if _loop.inner.touch.role == Role::Client {
        client::frame(_loop);
    }
}
