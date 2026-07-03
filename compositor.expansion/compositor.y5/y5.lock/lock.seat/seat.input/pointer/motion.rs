use smithay::backend::input::{AbsolutePositionEvent, Event, InputBackend, PointerMotionEvent};
use smithay::input::pointer::{MotionEvent, PointerHandle, RelativeMotionEvent};
use smithay::utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER, Serial, Size};
use compositor_y5_camera_transform_translate::translate;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_y5_surface_interface_base::hit::{self, SurfaceHit};
use compositor_monitor_compositor_iced_base::HandleId;

pub fn absolute<I: InputBackend>(
    event: &<I as InputBackend>::PointerMotionAbsoluteEvent,
    _loop: &mut Loop,
) {
    let ctx = _loop.size_ctx_all();
    let physical_size_as_logical = smithay::utils::Size::<i32, Logical>::from((
        ctx.screen_size_physical.0.round() as i32,
        ctx.screen_size_physical.1.round() as i32,
    ));

    let raw_pos: Point<f64, Logical> = event.position_transformed(physical_size_as_logical);
    let position_screen = Point::<f64, Physical>::from((raw_pos.x, raw_pos.y));

    let t: Transform = (position_screen, ctx).into();
    let position_normalized = &t.into_storage_point_f64();

    let position_normalized = position_normalized.clone().into();
    let serial = SERIAL_COUNTER.next_serial();
    let pointer = _loop.state.seat.seat.get_pointer().unwrap();

    dispatch(
        _loop,
        event.time_msec(),
        serial,
        pointer,
        position_normalized,
        None,
    );
}

pub fn relative<I: InputBackend>(
    event: &<I as InputBackend>::PointerMotionEvent,
    _loop: &mut Loop,
) {
    let ctx = _loop.size_ctx_all();
    let dt = event.delta();
    let previous_phys = _loop.inner.pointer_mut().motion.clone();

    _loop.inner.pointer_mut().motion.x += dt.x;
    _loop.inner.pointer_mut().motion.y += dt.y;

    let final_world: Point<f64, Logical> = {
        let (pw, ph) = ctx.screen_size_physical;
        _loop.inner.pointer_mut().motion.x = _loop.inner.pointer_mut().motion.x.clamp(0.0, pw);
        _loop.inner.pointer_mut().motion.y = _loop.inner.pointer_mut().motion.y.clamp(0.0, ph);

        let pt = Point::<f64, Physical>::from((
            _loop.inner.pointer_mut().motion.x,
            _loop.inner.pointer_mut().motion.y,
        ));
        let t: Transform = (pt, ctx).into();
        t.into_storage_point_f64()
    };
    let position_normalized = final_world;

    let position_normalized = position_normalized;

    let serial = SERIAL_COUNTER.next_serial();
    let pointer = _loop.state.seat.seat.get_pointer().unwrap();

    dispatch(
        _loop,
        event.time_msec(),
        serial,
        pointer,
        position_normalized,
        Some(dt),
    );
}

pub fn dispatch(
    _loop: &mut Loop,
    event_time: u32,
    serial: Serial,
    pointer: PointerHandle<Dispatch>,
    position_normalized: Point<f64, Logical>,
    delta: Option<(Point<f64, Logical>)>,
) {
    let under = compositor_y5_surface_interface_base::hit::surface_under_filtered(
        _loop,
        position_normalized,
        &|hit| {
            let Some(iced_layer) = hit.iced_layer() else {
                return false;
            };
            (iced_layer & compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits()) != 0
        },
    );

    let iced_target = under.as_ref().and_then(|h| h.iced_handle());
    let iced_screen_point = match under.as_ref().and_then(|h| h.screen_point()) {
        Some(p) => p,
        None => {
            let ctx = _loop.size_ctx_all();
            let t: Transform = (position_normalized, ctx).into();
            let p: Point<f64, Physical> = t.into();
            p
        }
    };

    let (iced_transform, iced_output_size) = hit::iced_camera(_loop);

    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        registry.route_pointer_to(
            iced_target,
            iced_screen_point,
            &iced_transform,
            iced_output_size,
        );
    }

    pointer.motion(
        &mut _loop.state,
        None,
        &MotionEvent {
            location: position_normalized,
            serial,
            time: event_time,
        },
    );

    pointer.frame(&mut _loop.state);
}
