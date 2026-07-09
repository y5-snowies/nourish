use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, InputBackend, InputEvent, KeyboardKeyEvent,
    PointerButtonEvent, Switch, SwitchState, SwitchToggleEvent,
};
use compositor_orchestration_core_state_base::Loop;

// /// Delegation of input events from the compositor seat loop
pub fn process_input_event<I: InputBackend>(_loop: &mut Loop, event: &InputEvent<I>) {
    match event {
        InputEvent::Keyboard { event, .. } => {
            compositor_y5_lock_seat_input::keyboard::keyboard::input_received::<I>(event, _loop);
        }
        InputEvent::PointerMotionAbsolute { event, .. } => {
            compositor_y5_lock_seat_input::pointer::motion::absolute::<I>(event, _loop);
        }
        InputEvent::PointerButton { event, .. } => {
            compositor_y5_lock_seat_input::pointer::button::button::<I>(event, _loop);
        }
        InputEvent::PointerAxis { event, .. } => {
            compositor_y5_lock_seat_input::pointer::axis::axis::<I>(event, _loop);
        }

        InputEvent::PointerMotion { event, .. } => {
            compositor_y5_lock_seat_input::pointer::motion::relative::<I>(event, _loop);
        }

        InputEvent::DeviceAdded { .. } => {}
        InputEvent::DeviceRemoved { .. } => {}
        InputEvent::GestureSwipeBegin { .. } => {}
        InputEvent::GestureSwipeUpdate { .. } => {}
        InputEvent::GestureSwipeEnd { .. } => {}
        InputEvent::GesturePinchBegin { .. } => {}
        InputEvent::GesturePinchUpdate { .. } => {}
        InputEvent::GesturePinchEnd { .. } => {}
        InputEvent::GestureHoldBegin { .. } => {}
        InputEvent::GestureHoldEnd { .. } => {}
        // Lock screen: single-finger pointer emulation against the lock handlers.
        InputEvent::TouchDown { event, .. } => {
            compositor_orchestration_seat_pointer_input::touch::aux::lock::down::<I>(event, _loop);
        }
        InputEvent::TouchMotion { event, .. } => {
            compositor_orchestration_seat_pointer_input::touch::aux::lock::motion::<I>(event, _loop);
        }
        InputEvent::TouchUp { event, .. } => {
            compositor_orchestration_seat_pointer_input::touch::aux::lock::up::<I>(event, _loop);
        }
        InputEvent::TouchCancel { .. } => {}
        InputEvent::TouchFrame { .. } => {}
        InputEvent::TabletToolAxis { .. } => {}
        InputEvent::TabletToolProximity { .. } => {}
        InputEvent::TabletToolTip { .. } => {}
        InputEvent::TabletToolButton { .. } => {}
        InputEvent::SwitchToggle { event, .. } => {
            if event.switch() == Some(Switch::Lid) {
                // libinput: switch On == lid closed. Lid policy applies even while
                // locked (close-to-suspend must still work on the lock screen).
                let lid_open = event.state() == SwitchState::Off;
                compositor_orchestration_seat_lid_policy::policy::on_lid_toggle(_loop, lid_open);
            }
        }
        InputEvent::Special(_) => {}
    }
}
