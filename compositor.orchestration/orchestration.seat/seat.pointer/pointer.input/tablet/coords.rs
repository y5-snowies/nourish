//! Pen absolute-position → y5-world resolution. Tablet tool events are
//! `AbsolutePositionEvent`s (like `PointerMotionAbsolute`), so this mirrors
//! `motion::absolute`'s physical→world step verbatim: never hand-roll output
//! projection (y5's pannable world). Returns the physical screen point (for pan
//! deltas) and the normalized world point (for hit-testing + client coords).

use smithay::backend::input::{AbsolutePositionEvent, InputBackend};
use smithay::utils::{Logical, Physical, Point, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_camera_transform_translate::transform::Transform;

pub fn world<I, E>(event: &E, _loop: &mut Loop) -> (Point<f64, Physical>, Point<f64, Logical>)
where
    I: InputBackend,
    E: AbsolutePositionEvent<I>,
{
    let screen = _loop.size_ctx_all();
    // position_transformed wants Size<_, Logical>; the panel's physical size is
    // passed wrapped as Logical and the result immediately re-tagged as Physical
    // (identical to `motion::absolute`).
    let physical_size_as_logical = Size::<i32, Logical>::from((
        screen.screen_size_physical.0.round() as i32,
        screen.screen_size_physical.1.round() as i32,
    ));
    let raw: Point<f64, Logical> = event.position_transformed(physical_size_as_logical);
    let screen_pt = Point::<f64, Physical>::from((raw.x, raw.y));
    let ctx = _loop.pointer_context(screen_pt);
    let t: Transform = (screen_pt, ctx).into();
    (screen_pt, t.into_storage_point_f64())
}
