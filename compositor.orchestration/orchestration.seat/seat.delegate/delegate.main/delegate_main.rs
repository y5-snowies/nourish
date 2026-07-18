use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent,
    GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyboardKeyEvent, PointerButtonEvent, Switch,
    SwitchState, SwitchToggleEvent,
};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_pointer_input::touch;

/// Delegation of input events from the compositor seat loop
pub fn process_input_event<I: InputBackend>(_loop: &mut Loop, event: &InputEvent<I>) {
    // Track the active input modality (touch vs real pointer / pen). Touch emulation
    // drives the pointer handlers directly and never routes through here, so these
    // arms are genuine device input. Using a real pointer / pen again also
    // auto-dismisses the sticky touch pane. `last_input_touch` lets modality-sensitive
    // UI (e.g. the selection-toolbar placement) follow the device actually in use.
    match event {
        InputEvent::TouchDown { .. }
        | InputEvent::TouchMotion { .. }
        | InputEvent::TouchUp { .. }
        | InputEvent::TouchCancel { .. }
        | InputEvent::TouchFrame { .. } => {
            _loop.inner.touch.last_input_touch = true;
        }
        InputEvent::PointerMotion { .. }
        | InputEvent::PointerMotionAbsolute { .. }
        | InputEvent::PointerButton { .. }
        | InputEvent::PointerAxis { .. } => {
            _loop.inner.touch.last_input_touch = false;
            _loop.inner.touch.pane_world = None;
        }
        InputEvent::TabletToolProximity { .. }
        | InputEvent::TabletToolAxis { .. }
        | InputEvent::TabletToolTip { .. }
        | InputEvent::TabletToolButton { .. } => {
            // The pen is a non-touch modality (so the selection toolbar drops its
            // sticky touch placement), but unlike the mouse it must NOT dismiss the
            // touch pane — the pen is allowed to operate it (feature parity).
            _loop.inner.touch.last_input_touch = false;
        }
        _ => {}
    }
    match event {
        InputEvent::Keyboard { event, .. } => {
            compositor_orchestration_seat_keyboard_input::keyboard::input_received::<I>(event, _loop);
        }
        InputEvent::PointerMotionAbsolute { event, .. } => {
            compositor_orchestration_seat_pointer_input::motion::absolute::<I>(event, _loop)
        }
        InputEvent::PointerButton { event, .. } => {
            compositor_orchestration_seat_pointer_input::button::button::<I>(event, _loop);
        }
        InputEvent::PointerAxis { event, .. } => {
            compositor_orchestration_seat_pointer_input::axis::axis::<I>(event, _loop);
        }

        InputEvent::PointerMotion { event, .. } => {
            compositor_orchestration_seat_pointer_input::motion::relative::<I>(event, _loop);
        }

        InputEvent::DeviceAdded { .. } => {}
        InputEvent::DeviceRemoved { .. } => {}
        InputEvent::GestureSwipeBegin { event, .. } => {
            _loop.inner.gesture.begin(event.fingers());
        }
        InputEvent::GestureSwipeUpdate { event, .. } => {
            _loop.inner.gesture.update(event.delta_x(), event.delta_y());
        }
        InputEvent::GestureSwipeEnd { event, .. } => {
            let cancelled = event.cancelled();
            _loop.inner.gesture.active = false;
            compositor_y5_canvas_input_gesture::gesture::swipe_end(_loop, cancelled);
        }
        // Two/three-finger pinch is a continuous canvas (or forwarded window) zoom;
        // a FOUR-finger pinch is a discrete window command (fit one / fit all),
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
        InputEvent::GestureHoldBegin { .. } => {}
        InputEvent::GestureHoldEnd { .. } => {}
        // Touch: the session router forwards to a client's `wl_touch`, emulates the
        // pointer, or runs a canvas gesture — decided per sequence.
        InputEvent::TouchDown { event, .. } => touch::session::down::<I>(event, _loop),
        InputEvent::TouchMotion { event, .. } => touch::session::motion::<I>(event, _loop),
        InputEvent::TouchUp { event, .. } => touch::session::up::<I>(event, _loop),
        InputEvent::TouchCancel { event, .. } => touch::session::cancel::<I>(event, _loop),
        InputEvent::TouchFrame { event, .. } => touch::session::frame::<I>(event, _loop),
        // Pen / stylus: native zwp_tablet_v2 to a tablet-aware client, else pan the
        // canvas. Role latched at tip-down (see seat.pointer tablet::session).
        InputEvent::TabletToolProximity { event, .. } => {
            compositor_orchestration_seat_pointer_input::tablet::proximity::proximity::<I>(event, _loop)
        }
        InputEvent::TabletToolAxis { event, .. } => {
            compositor_orchestration_seat_pointer_input::tablet::axis::axis::<I>(event, _loop)
        }
        InputEvent::TabletToolTip { event, .. } => {
            compositor_orchestration_seat_pointer_input::tablet::tip::tip::<I>(event, _loop)
        }
        InputEvent::TabletToolButton { event, .. } => {
            compositor_orchestration_seat_pointer_input::tablet::button::button::<I>(event, _loop)
        }
        InputEvent::SwitchToggle { event, .. } => {
            if event.switch() == Some(Switch::Lid) {
                // libinput: switch On == lid closed.
                let lid_open = event.state() == SwitchState::Off;
                compositor_orchestration_seat_lid_policy::policy::on_lid_toggle(_loop, lid_open);
            }
        }
        InputEvent::Special(_) => {}
    }
}
