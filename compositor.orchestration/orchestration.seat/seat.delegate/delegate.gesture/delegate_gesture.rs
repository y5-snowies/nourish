//! Touchpad swipe / pinch arms of the main seat delegate.
//!
//! Split out of `delegate.main` so that stays a flat router: these arms carry real
//! policy (the finger-count split between a continuous canvas zoom and a discrete
//! window command), which is the only place in the delegate that decides rather than
//! forwards.

use compositor_orchestration_core_state_base::Loop;
use smithay::backend::input::{
    GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent,
    InputBackend, InputEvent,
};

/// Handle a gesture event. Returns `false` if `event` was not a gesture, so the
/// caller can continue matching.
pub fn process_input_event<I: InputBackend>(_loop: &mut Loop, event: &InputEvent<I>) -> bool {
    match event {
        InputEvent::GestureSwipeBegin { event, .. } => _loop.inner.gesture.begin(event.fingers()),
        InputEvent::GestureSwipeUpdate { event, .. } => {
            _loop.inner.gesture.update(event.delta_x(), event.delta_y());
        }
        InputEvent::GestureSwipeEnd { event, .. } => {
            let cancelled = event.cancelled();
            _loop.inner.gesture.active = false;
            compositor_y5_canvas_input_gesture::gesture::swipe_end(_loop, cancelled);
        }
        // Two/three-finger pinch is a continuous canvas (or forwarded window) zoom; a
        // FOUR-finger pinch is a discrete window command (fit one / fit all),
        // accumulated here and dispatched to the y5 handler at end.
        InputEvent::GesturePinchBegin { event, .. } => {
            let fingers = event.fingers();
            _loop.inner.gesture.pinch_fingers = fingers;
            if fingers >= 4 {
                _loop.inner.gesture.pinch_scale = 1.0;
            } else {
                compositor_orchestration_seat_pointer_input::pinch::begin::<I>(event, _loop);
            }
        }
        InputEvent::GesturePinchUpdate { event, .. } => {
            if _loop.inner.gesture.pinch_fingers >= 4 {
                _loop.inner.gesture.pinch_scale = event.scale();
            } else {
                compositor_orchestration_seat_pointer_input::pinch::update::<I>(event, _loop);
            }
        }
        InputEvent::GesturePinchEnd { event, .. } => {
            if _loop.inner.gesture.pinch_fingers >= 4 {
                let scale = _loop.inner.gesture.pinch_scale;
                _loop.inner.gesture.pinch_fingers = 0;
                compositor_y5_canvas_input_gesture::gesture::pinch_four(_loop, scale);
            } else {
                compositor_orchestration_seat_pointer_input::pinch::end::<I>(event, _loop);
            }
        }
        InputEvent::GestureHoldBegin { .. } | InputEvent::GestureHoldEnd { .. } => {}
        _ => return false,
    }
    true
}
