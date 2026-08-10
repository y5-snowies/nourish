//! Make the system cursor follow the pen AND let the pen drive the canvas.
//!
//! Like the mouse's absolute-motion path (`motion::absolute`), we offer the motion to
//! the world input bus FIRST — so the pen drives the canvas PAN (`position_updating` /
//! Hand grab) and the CanvasSystem MOVE/SCALE/SELECTBOX transforms exactly like a
//! mouse — and only on `Pass` fall through to the normal pointer dispatch (which moves
//! the cursor + gives whatever is under it pointer focus). Routing on EVERY pen motion
//! (hover included) keeps the camera's `position_previous` current, so the first pan
//! step has a correct delta. Native tablet events (forwarded separately) sit on top of
//! this for tablet-aware apps.

use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_support_system_input_event_base::base::{InputEvent, InputFlow};

pub fn follow(_loop: &mut Loop, screen: Point<f64, Physical>, world: Point<f64, Logical>, time: u32) {
    // The `pointer_state` descriptor. This path MIRRORS `motion::absolute` rather
    // than calling it, so anything published there has to be published here too —
    // otherwise a pen moves the cursor and a shader watching the cursor never
    // notices. Before the world bus, so a consumed motion (a canvas drag) still
    // reports where the pen is.
    let ctx = _loop.pointer_context(screen);
    crate::motion::publish_pointer(&ctx, screen);
    // World bus first — a canvas transform (MOVE/SCALE/SELECTBOX) or separator/float
    // drag Consumes; a pan Passes (non-intercepting) but still advances the camera.
    let ev = InputEvent::PointerMotion {
        x: world.x,
        y: world.y,
        screen_x: screen.x,
        screen_y: screen.y,
        delta_x: 0.0,
        delta_y: 0.0,
    };
    if compositor_orchestration_input_drive_base::drive::route(_loop, ev) == InputFlow::Consume {
        return;
    }
    let Some(pointer) = _loop.state.seat.seat.get_pointer() else { return };
    let serial = SERIAL_COUNTER.next_serial();
    crate::native_motion::dispatch::dispatch(_loop, time, serial, pointer, world, None, false);
}
