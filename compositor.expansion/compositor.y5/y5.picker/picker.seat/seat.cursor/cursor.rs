//! Keep the seat cursor location in sync with the picker pointer so the rendered
//! pointer follows it, AND feed the cursor to the details panel (iced) so it
//! tracks hovers/clicks. Screen → world round-trip (like the lock screen).

use smithay::input::pointer::MotionEvent;
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_y5_surface_interface_base::hit::{self, surface_under_filtered};

pub fn update(state: &mut Loop, screen_x: f64, screen_y: f64) {
    let ctx = state.size_ctx_all();
    let position_screen = Point::<f64, Physical>::from((screen_x, screen_y));
    let t: Transform = (position_screen, ctx).into();
    let world: Point<f64, Logical> = t.into_storage_point_f64().into();

    // Route the cursor to the iced details panel (CursorEntered/Moved) so its
    // widgets light up and clicks land — the picker bypasses the normal motion.
    let under = surface_under_filtered(state, world, &|h| {
        h.iced_layer().map(|l| (l & Layer::PICKER_SCENE.bits()) != 0).unwrap_or(false)
    });
    let target = under.as_ref().and_then(|h| h.iced_handle());
    let screen_point = under.as_ref().and_then(|h| h.screen_point()).unwrap_or(position_screen);
    let (transform, output_size) = hit::iced_camera(state);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.route_pointer_to(target, screen_point, &transform, output_size);
    }

    let serial = SERIAL_COUNTER.next_serial();
    let pointer = state.state.seat.seat.get_pointer().unwrap();
    pointer.motion(&mut state.state, None, &MotionEvent { location: world, serial, time: 0 });
    pointer.frame(&mut state.state);
}
