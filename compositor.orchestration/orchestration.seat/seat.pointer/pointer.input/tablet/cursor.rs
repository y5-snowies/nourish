//! Make the system cursor follow the pen. We drive the very same pointer dispatch
//! the mouse uses (`native_motion::dispatch`) with the pen's absolute world point,
//! so the cursor moves and whatever is under it gets normal pointer focus — native
//! tablet events (forwarded separately) sit on top of this for tablet-aware apps,
//! and non-tablet apps get a usable mouse pointer from the pen.

use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;

pub fn follow(_loop: &mut Loop, world: Point<f64, Logical>, time: u32) {
    let Some(pointer) = _loop.state.seat.seat.get_pointer() else { return };
    let serial = SERIAL_COUNTER.next_serial();
    crate::native_motion::dispatch::dispatch(_loop, time, serial, pointer, world, None, false);
}
