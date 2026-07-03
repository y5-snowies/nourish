use smithay::input::pointer::{MotionEvent, PointerHandle, RelativeMotionEvent};
use smithay::utils::{Logical, Physical, Point, Serial};
use compositor_y5_camera_transform_translate::translate;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_orchestration_core_state_base::state::{CoordinateTrait, Orchestrator as State};
use compositor_y5_surface_interface_base::hit::SurfaceHit;
use compositor_y5_surface_interface_base::hit::{self, surface_under_filtered};
use compositor_y5_window_interface_draw::visible::DrawWindow;

pub fn dispatch(
    _loop: &mut Loop,
    event_time: u32,
    serial: Serial,
    pointer: PointerHandle<Dispatch>,
    position_normalized: Point<f64, Logical>,
    delta: Option<(Point<f64, Logical>, Point<f64, Logical>)>,
    was_constrain_locked: bool,
) {
    // Overview overlay open → windows are presentational: reject window hits so
    // pointer focus never enters a window (iced screen surfaces still match).
    let overview_open = _loop.inner.overview().visible;
    let under = surface_under_filtered(_loop, position_normalized, &|hit| {
        if let Some(window) = hit.window() {
            return !overview_open && window.visible(_loop);
        };

        true
    });

    let compositor_output_size = _loop
        .inner.space_state()
        .state
        .output_geometry(_loop.inner.space_state().state.outputs().next().unwrap())
        .unwrap()
        .size;

    let iced_target = under.as_ref().and_then(|h| h.iced_handle());
    let iced_screen_point = match under.as_ref().and_then(|h| h.screen_point()) {
        Some(p) => p,
        None => {
            // Pane context → TRUE physical cursor (screen-space iced lives in
            // physical pixels; full-output projection would be wrong when split).
            let ctx = _loop.focus_pane_context();
            let t: compositor_y5_camera_transform_translate::transform::Transform =
                (position_normalized, ctx).into();
            let p: Point<f64, Physical> = t.into();
            p
        }
    };

    let mut under_hit: Option<_> = None;

    if let Some(under) = under {
        match under {
            SurfaceHit::Iced {
                handle,
                screen_point,
                ..
            } => {}
            SurfaceHit::Window {
                surface, position, ..
            }
            | SurfaceHit::Layer {
                surface,
                position_space: position,
                ..
            } => {
                // Make sure it is acknowledged for pointer motion.
                under_hit = Some((surface, position))
            }
        }
    }

    let (iced_transform, iced_output_size) = hit::iced_camera(_loop);

    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        registry.route_pointer_to(
            iced_target,
            iced_screen_point,
            &iced_transform,
            iced_output_size,
        );
    }

    // CHECK: Improve to not use cloning and for the latter call.(the focus check)

    let prev_focus = pointer.current_focus();

    if !was_constrain_locked {
        pointer.motion(
            &mut _loop.state,
            under_hit.clone(),
            &MotionEvent {
                location: position_normalized,
                serial,
                time: event_time,
            },
        );
    }

    if let Some((delta, delta_unaccelerated)) = delta {
        // tracing::info!(
        //     ?delta,
        //     ?delta_unaccelerated,
        //     "relative_motion firing: {:?}",
        //     under_hit.is_some()
        // );

        pointer.relative_motion(
            &mut _loop.state,
            under_hit.clone(),
            &RelativeMotionEvent {
                delta,
                delta_unaccel: delta_unaccelerated,
                utime: event_time as u64 * 1000, // RelativeMotionEvent uses microseconds
            },
        );
    } else {
        // tracing::warn!("delta was None — relative_motion skipped");
    }

    pointer.frame(&mut _loop.state);

    let new_focus = under_hit.map(|(target, _)| target);

    if prev_focus.as_ref() != new_focus.as_ref() {
        if let Some(token) = _loop.state.seat.reevaluate_pointer_constraints(
            &pointer,
            prev_focus.as_ref(),
            new_focus.as_ref(),
        ) {
            _loop.apply_constraint_restoration(token);
        }
    }
}
