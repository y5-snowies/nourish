//! Keep the seat cursor location in sync with the picker pointer so the rendered
//! pointer follows it, AND feed the cursor to the details panel (iced) so it
//! tracks hovers/clicks.
//!
//! The panel is hit in SCREEN space against the picker's own registry — it is
//! composed at 1:1 with no camera, so the incoming physical point is already the
//! right basis and no world round-trip is involved. Only the SEAT pointer still
//! goes through the session context, because that is what `pointer.draw` projects
//! back when it renders the sprite; the two must agree.

use smithay::input::pointer::MotionEvent;
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};

pub fn update(state: &mut Loop, screen_x: f64, screen_y: f64) {
    // No warp is in effect here — the picker draws its own scene, not a pipeline
    // pass that displaces content. Say so, because the slot is a SEAT-wide latch:
    // it is written on every SPATIAL motion (`pointer.input/motion.rs`) and by
    // nothing else, so a bundle that was warping when the picker opened leaves it
    // `Some(stale)` for the whole session. `pointer.draw` reads it in preference to
    // `current_location()`, so the sprite freezes at wherever the hand last was on
    // the desktop — projected through the session camera, which can put it right
    // off the screen. Every path that owns pointer motion has to publish this.
    compositor_orchestration_seat_pointer_publish::publish::set_true_screen(None);

    let position_screen = Point::<f64, Physical>::from((screen_x, screen_y));
    // Hover/enter/leave for the details panel, in the space it is drawn in.
    compositor_y5_picker_seat_iced::iced::route_motion(state, position_screen);

    let ctx = state.size_ctx_all();
    let t: Transform = (position_screen, ctx).into();
    let world: Point<f64, Logical> = t.into_storage_point_f64().into();
    let serial = SERIAL_COUNTER.next_serial();
    let pointer = state.state.seat.seat.get_pointer().unwrap();
    pointer.motion(&mut state.state, None, &MotionEvent { location: world, serial, time: 0 });
    pointer.frame(&mut state.state);
}
