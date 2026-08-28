use compositor_orchestration_smithay_data_base::data as kernel_data;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_system_trait_system_base::base::SystemCx;
use compositor_y5_camera_transform_state::state::Transform;
use compositor_y5_navigator_state_base::state::NavigatorOutput;
use smithay::input::pointer::MotionEvent;
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use std::time::{SystemTime, UNIX_EPOCH};

/// Apply the warp intent directly to the seat, keeping the cursor visually
/// still while the camera eases. Runs on the update path, where the navigator
/// system holds the seat disjointly (`cx.seat` downcasts to `Dispatch`) — so
/// there is no `pending_pointer_warp` round-trip through the frame driver
/// (document/SMITHAY_DECOUPLING.md "P3"). No-op when there is no warp this tick.
pub fn apply_warp(cx: &mut SystemCx, warp: Option<(f64, f64)>) {
    let Some((x, y)) = warp else { return };
    let Some(dispatch) = cx.seat.as_deref_mut().and_then(|s| s.downcast_mut::<Dispatch>()) else {
        return;
    };
    let Some(pointer) = dispatch.seat.seat.get_pointer() else { return };
    let serial = SERIAL_COUNTER.next_serial();
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0);
    pointer.motion(dispatch, None, &MotionEvent { location: Point::from((x, y)), serial, time });
    pointer.frame(dispatch);
}

/// Keep the cursor visually still while the camera moves: re-project the
/// pointer's world position from the PREVIOUS camera to the eased one.
pub fn warp_intent(
    cx: &SystemCx,
    previous: &Transform,
    output: &NavigatorOutput,
) -> Option<(f64, f64)> {
    let screen = cx.kernel.try_get(&kernel_data::SCREEN)?.clone();
    let pointer = cx.kernel.try_get(&kernel_data::POINTER)?;
    let position_world: Point<f64, Logical> = pointer.current_location();

    let eased_position = output.position.unwrap_or((previous.position.x, previous.position.y));
    let eased_zoom = output.zoom.unwrap_or(*previous.zoom());

    let screen_size = (screen.size.w as f64, screen.size.h as f64);
    let ctx_prev = compositor_y5_camera_transform_translate::transform::Context::new(
        (previous.position.x, previous.position.y),
        *previous.zoom(),
        screen_size,
        screen.scale,
    );
    let ctx_new = compositor_y5_camera_transform_translate::transform::Context::new(
        eased_position,
        eased_zoom,
        screen_size,
        screen.scale,
    );

    // world -> screen under the previous camera, screen -> world under the new
    let prev: compositor_y5_camera_transform_translate::transform::Transform =
        (position_world, ctx_prev).into();
    let screen_phys: Point<f64, Physical> = prev.into();
    let new: compositor_y5_camera_transform_translate::transform::Transform =
        (screen_phys, ctx_new).into();
    let warped: Point<f64, Logical> = new.into_storage_point_f64();
    Some((warped.x, warped.y))
}
