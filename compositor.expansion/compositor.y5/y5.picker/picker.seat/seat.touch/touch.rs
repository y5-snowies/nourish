//! Single-finger pointer emulation against the WORLD PICKER's own pointer handlers.
//!
//! A sibling of `seat.keyboard` / `seat.pointer` so every input class the picker
//! accepts has one home, and `seat.dispatch` stays a flat event -> handler table.
//! The picker is compositor UI, not a client surface, so there is no `wl_touch`
//! forwarding and no gestures: down = move + press, motion = move, up = release.
//! The backend-generic half of the emulation is reused from orchestration
//! (`touch::aux`).

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_pointer_input::touch::aux::{self, TouchEmu};
use compositor_y5_picker_seat_pointer::pointer::{absolute, button};
use smithay::backend::input::InputBackend;

pub fn down<I: InputBackend>(event: &I::TouchDownEvent, state: &mut Loop) {
    aux::down::<I>(event, state, absolute::<TouchEmu>, button::<TouchEmu>);
}

pub fn motion<I: InputBackend>(event: &I::TouchMotionEvent, state: &mut Loop) {
    aux::motion::<I>(event, state, absolute::<TouchEmu>);
}

pub fn up<I: InputBackend>(event: &I::TouchUpEvent, state: &mut Loop) {
    aux::release::<I, _>(event, state, button::<TouchEmu>);
}
