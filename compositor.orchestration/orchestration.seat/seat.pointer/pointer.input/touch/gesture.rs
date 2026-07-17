//! Multi-finger gesture engine. Synthesizes the trackpad's gestures from the raw
//! touch contacts and drives the *existing* handlers, so touch and touchpad share
//! one code path: 2 fingers → pinch-zoom + pan, 3 → directional swipe, 4 → fit.
use super::backend::{PinchEvent, TouchEmu};
use super::{emulate, geom};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_gesture_touch::touch::{Mode, TouchMode};

/// 2-finger-tap slop (physical px of centroid drift) and pinch tolerance (|scale-1|)
/// beyond which the gesture is a pan/pinch, not a tap → cancels the right click.
const RTAP_SLOP: f64 = 24.0;
const RTAP_PINCH: f64 = 0.12;

/// Enter a gesture mode: latch the reference spread + centroid and open the
/// matching native gesture (pinch for zoom, swipe accumulator for 3-finger).
pub fn begin(_loop: &mut Loop, mode: Mode, time: u32) {
    _loop.inner.touch.base_spread = _loop.inner.touch.spread().max(1.0);
    _loop.inner.touch.prev_centroid = _loop.inner.touch.centroid();
    _loop.inner.touch.fit_scale = 1.0;
    match mode {
        // Pointer AND Hand modes keep the pinch on the CANVAS (camera zoom) and never
        // forward it to the window under the fingers — a 2-finger pinch/pan zooms & pans
        // the y5-world. Hand needs this explicitly now that it no longer arms the canvas
        // Hand grab (the camera's `canvas_owns_gesture` used to key off that grab; with
        // Hand panning via glide instead, we must latch canvas ownership here). A
        // canvas-owned pinch needs no window begin/end, so we just set the state that
        // `pinch::update`/`end` read (mirroring `pinch::begin`'s canvas branch).
        Mode::Zoom
            if matches!(
                _loop.inner.touch.tool_mode,
                TouchMode::Pointer | TouchMode::Hand
            ) =>
        {
            _loop.inner.gesture.pinch_to_window = false;
            _loop.inner.gesture.pinch_prev_scale = 1.0;
        }
        Mode::Zoom => crate::pinch::begin::<TouchEmu>(&pinch(time, 1.0, 0.0, 0.0, false), _loop),
        Mode::Swipe => _loop.inner.gesture.begin(3),
        Mode::Fit | Mode::None => {}
    }
}

/// Drive the current mode from the latest contact positions.
pub fn update(_loop: &mut Loop, time: u32) {
    let centroid = _loop.inner.touch.centroid();
    let prev = _loop.inner.touch.prev_centroid;
    let (dx, dy) = (centroid.x - prev.x, centroid.y - prev.y);
    _loop.inner.touch.prev_centroid = centroid;
    // A 2-finger tap (pointer-mode right click) is disqualified the moment the
    // fingers pan or pinch past a small slop. Only checked with both fingers down
    // (a single remaining finger has no meaningful spread).
    if _loop.inner.touch.rtap_candidate && _loop.inner.touch.len() == 2 {
        let origin = _loop.inner.touch.rtap_origin;
        let drift = (centroid.x - origin.x).hypot(centroid.y - origin.y);
        let scale = _loop.inner.touch.spread() / _loop.inner.touch.base_spread.max(1.0);
        if drift > RTAP_SLOP || (scale - 1.0).abs() > RTAP_PINCH {
            _loop.inner.touch.rtap_candidate = false;
        }
    }
    match _loop.inner.touch.mode {
        Mode::Zoom => {
            let scale = _loop.inner.touch.spread() / _loop.inner.touch.base_spread;
            // Anchor zoom/pan at the pinch centre by warping the pointer there
            // (the canvas-zoom + scroll handlers key off the pointer location).
            let (nx, ny) = geom::fraction_of(_loop, centroid);
            emulate::move_to(_loop, nx, ny, time);
            crate::pinch::update::<TouchEmu>(&pinch(time, scale, dx, dy, false), _loop);
            // Canvas-owned pinch → also pan via the centroid delta; a window-
            // forwarded pinch already receives the centre delta itself. A 2-finger
            // pan is STRICT (no momentum) — `pan(.., false)`.
            if !_loop.inner.gesture.pinch_to_window && (dx != 0.0 || dy != 0.0) {
                emulate::pan(_loop, dx, dy, false);
            }
        }
        Mode::Swipe => _loop.inner.gesture.update(dx, dy),
        Mode::Fit => {
            _loop.inner.touch.fit_scale = _loop.inner.touch.spread() / _loop.inner.touch.base_spread;
        }
        Mode::None => {}
    }
}

/// Close the current mode out: end the native pinch, fire the swipe, or apply the
/// four-finger fit command.
pub fn end(_loop: &mut Loop, mode: Mode, time: u32, cancelled: bool) {
    match mode {
        Mode::Zoom => crate::pinch::end::<TouchEmu>(&pinch(time, 1.0, 0.0, 0.0, cancelled), _loop),
        Mode::Swipe => {
            _loop.inner.gesture.active = false;
            compositor_y5_canvas_input_gesture::gesture::swipe_end(_loop, cancelled);
        }
        Mode::Fit => {
            if !cancelled {
                let scale = _loop.inner.touch.fit_scale;
                compositor_y5_canvas_input_gesture::gesture::pinch_four(_loop, scale);
            }
        }
        Mode::None => {}
    }
}

fn pinch(time: u32, scale: f64, dx: f64, dy: f64, cancelled: bool) -> PinchEvent {
    PinchEvent { time, fingers: 2, scale, dx, dy, cancelled }
}
