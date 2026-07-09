//! Multi-finger gesture engine. Synthesizes the trackpad's gestures from the raw
//! touch contacts and drives the *existing* handlers, so touch and touchpad share
//! one code path: 2 fingers → pinch-zoom + pan, 3 → directional swipe, 4 → fit.
use super::backend::{AxisEvent, PinchEvent, TouchEmu};
use super::{emulate, geom};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_gesture_touch::touch::Mode;

/// Enter a gesture mode: latch the reference spread + centroid and open the
/// matching native gesture (pinch for zoom, swipe accumulator for 3-finger).
pub fn begin(_loop: &mut Loop, mode: Mode, time: u32) {
    _loop.inner.touch.base_spread = _loop.inner.touch.spread().max(1.0);
    _loop.inner.touch.prev_centroid = _loop.inner.touch.centroid();
    _loop.inner.touch.fit_scale = 1.0;
    match mode {
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
    match _loop.inner.touch.mode {
        Mode::Zoom => {
            let scale = _loop.inner.touch.spread() / _loop.inner.touch.base_spread;
            // Anchor zoom/pan at the pinch centre by warping the pointer there
            // (the canvas-zoom + scroll handlers key off the pointer location).
            let (nx, ny) = geom::fraction_of(_loop, centroid);
            emulate::move_to(_loop, nx, ny, time);
            crate::pinch::update::<TouchEmu>(&pinch(time, scale, dx, dy, false), _loop);
            // Canvas-owned pinch → also pan via a synthetic finger-scroll; a
            // window-forwarded pinch already receives the centre delta itself.
            if !_loop.inner.gesture.pinch_to_window && (dx != 0.0 || dy != 0.0) {
                crate::axis::axis::<TouchEmu>(&AxisEvent { time, horizontal: dx, vertical: dy }, _loop);
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
