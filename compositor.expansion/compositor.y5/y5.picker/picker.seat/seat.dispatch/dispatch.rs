//! Routes seat events to the picker keyboard / pointer handlers while the
//! picker overlay world is active (called from the seat delegate).

use smithay::backend::input::{InputBackend, InputEvent};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_pointer_input::touch::aux::{self, TouchEmu};
use compositor_y5_picker_seat_pointer::pointer::{absolute, button};

pub fn process_input_event<I: InputBackend>(state: &mut Loop, event: &InputEvent<I>) {
    match event {
        // Touch on the picker is single-finger pointer emulation against the picker's
        // OWN handlers (it is compositor UI, so no wl_touch / gestures). Handled here
        // rather than in the seat delegate so this crate stays the single place that
        // knows the picker's event -> handler table.
        InputEvent::TouchDown { event, .. } => {
            aux::down::<I>(event, state, absolute::<TouchEmu>, button::<TouchEmu>);
        }
        InputEvent::TouchMotion { event, .. } => {
            aux::motion::<I>(event, state, absolute::<TouchEmu>);
        }
        InputEvent::TouchUp { event, .. } => {
            aux::release::<I, _>(event, state, button::<TouchEmu>);
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
