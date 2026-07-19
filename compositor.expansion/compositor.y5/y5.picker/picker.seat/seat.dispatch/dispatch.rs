//! Routes seat events to the picker keyboard / pointer handlers while the
//! picker overlay world is active (called from the seat delegate).

use smithay::backend::input::{InputBackend, InputEvent};
use compositor_orchestration_core_state_base::Loop;
pub fn process_input_event<I: InputBackend>(state: &mut Loop, event: &InputEvent<I>) {
    match event {
        // Touch is single-finger pointer emulation against the picker's own handlers
        // (compositor UI — no wl_touch, no gestures); it lives in `picker.seat/seat.touch`
        // beside the keyboard and pointer handlers, so this stays a flat routing table.
        InputEvent::TouchDown { event, .. } => {
            compositor_y5_picker_seat_touch::touch::down::<I>(event, state);
        }
        InputEvent::TouchMotion { event, .. } => {
            compositor_y5_picker_seat_touch::touch::motion::<I>(event, state);
        }
        InputEvent::TouchUp { event, .. } => {
            compositor_y5_picker_seat_touch::touch::up::<I>(event, state);
        }
        InputEvent::Keyboard { event, .. } => {
            compositor_y5_picker_seat_keyboard::keyboard::input_received::<I>(event, state);
        }
        InputEvent::PointerButton { event, .. } => {
            compositor_y5_picker_seat_pointer::pointer::button::<I>(event, state);
        }
        InputEvent::PointerAxis { event, .. } => {
            compositor_y5_picker_seat_pointer::pointer::axis::<I>(event, state);
        }
        InputEvent::PointerMotionAbsolute { event, .. } => {
            compositor_y5_picker_seat_pointer::pointer::absolute::<I>(event, state);
        }
        InputEvent::PointerMotion { event, .. } => {
            compositor_y5_picker_seat_pointer::pointer::relative::<I>(event, state);
        }
        _ => {}
    }
}
