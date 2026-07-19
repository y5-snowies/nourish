//! Single-finger pointer emulation against the LOCK SCREEN's own pointer handlers.
//!
//! Lives here, beside `pointer` and `keyboard`, so the lock screen owns its whole
//! input surface: the seat delegate routes every event class into this crate and
//! never needs to know how lock touch is implemented. The backend-generic half of
//! the emulation is reused from orchestration (`touch::aux`).

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_pointer_input::touch::aux::{self, TouchEmu};
use smithay::backend::input::InputBackend;

use crate::pointer::{button::button, motion::absolute};

pub fn down<I: InputBackend>(event: &I::TouchDownEvent, _loop: &mut Loop) {
    aux::down::<I>(event, _loop, absolute::<TouchEmu>, button::<TouchEmu>);
}

pub fn motion<I: InputBackend>(event: &I::TouchMotionEvent, _loop: &mut Loop) {
    aux::motion::<I>(event, _loop, absolute::<TouchEmu>);
}

pub fn up<I: InputBackend>(event: &I::TouchUpEvent, _loop: &mut Loop) {
    aux::release::<I, _>(event, _loop, button::<TouchEmu>);
}
