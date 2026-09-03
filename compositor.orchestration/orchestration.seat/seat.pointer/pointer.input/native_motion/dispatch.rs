use smithay::input::pointer::{MotionEvent, PointerHandle, RelativeMotionEvent};
use smithay::utils::{Logical, Physical, Point, Serial};
use compositor_y5_camera_transform_translate::translate;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_orchestration_core_state_base::state::{CoordinateTrait, Orchestrator as State};
use compositor_y5_surface_interface_base::hit::SurfaceHit;
use compositor_y5_surface_interface_base::hit::{self, surface_under_filtered};
use compositor_y5_window_interface_draw::visible::DrawWindow;
use compositor_support_smithay_state_window_find::find;

pub fn dispatch(
    _loop: &mut Loop,
    event_time: u32,
    serial: Serial,
    pointer: PointerHandle<Dispatch>,
    position_normalized: Point<f64, Logical>,
    delta: Option<(Point<f64, Logical>, Point<f64, Logical>)>,
    was_constrain_locked: bool,
) {
    // Overview overlay open → windows AND wlr layer surfaces are presentational: reject both
    // so pointer focus never enters them (iced screen surfaces — the overview UI — still match).
    let overview_open = _loop.inner.overview().visible;
    // A window carried by `xdg_toplevel_drag_v1` is excluded from the hit test:
    // it tracks the cursor, so it would otherwise be the answer to every hit and
    // the drop target underneath would never be reached. Computed before the
    // filter so the closure does not also borrow `_loop`.
    let carried = _loop.state.toplevel_drag.carried_surface();
    let under = surface_under_filtered(_loop, position_normalized, &|hit| {
        if overview_open && (hit.window().is_some() || hit.is_layer()) {
            return false;
        }
        if let (Some(carried), Some(window)) = (carried.as_ref(), hit.window()) {
            if find::is_surface(window, carried) {
                return false;
            }
        }
        if let Some(window) = hit.window() {
            return window.visible(_loop);
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
            // Letterbox bars. Leaving `under_hit` unset gives the pointer no
            // focus, so smithay sends the previously-focused client a leave —
            // which is the truth: the cursor is over compositor pixels, not over
            // the client. Delivering an edge coordinate instead would have it
            // tracking a pointer that had left it.
            SurfaceHit::WindowChrome { .. } => {}
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

    // Under the hand tool the canvas owns the pointer: the cursor still moves
    // (that is what `motion` is for) but no client is its focus — the one under
    // it gets `leave` when the tool engages and `enter` when it is released, the
    // same shape as a compositor grab, and consistent with the withheld button
    // and relative motion. Compositor iced above keeps its hover.
    let hand = crate::constraint::hand_active(_loop);
    // BEFORE the enter goes out. If the surface the pointer is moving onto belongs to an
    // X11 window, that window has to be top of the X STACK first: the X server hit-tests
    // its own tree at the coordinate Xwayland derives, so a raise that lands after the
    // enter leaves the first events of a crossing routed against the old order — which is
    // how this failed when it was tried on the focus-change branch below. See
    // `Dispatch::raise_x11_for_pointer`; xwayland-satellite raises in the same breath as
    // its enter for the same reason. Passed as a plain surface — whether it is X11 at all
    // is not the rim's business — and a no-op for wayland windows.
    //
    // Only on a CROSSING, not every motion. The raise is a request written and flushed on
    // the X socket (never waited for — see `X11Wm::raise_window` for why a round trip
    // here would be a hang), and re-issuing it for every motion event would put one on
    // the input path at pointer frequency to re-assert something already true.
    let entering = under_hit.as_ref().map(|(surface, _)| surface.clone());
    if !hand && entering != prev_focus {
        _loop.state.raise_x11_for_pointer(entering.as_ref());
    }
    if !was_constrain_locked {
        pointer.motion(
            &mut _loop.state,
            if hand { None } else { under_hit.clone() },
            &MotionEvent {
                location: position_normalized,
                serial,
                time: event_time,
            },
        );
    }

    // Not under the hand tool: the deltas are the canvas pan, and a game that was
    // locked a moment ago must not keep turning with them.
    if let Some((delta, delta_unaccelerated)) = delta.filter(|_| !hand) {
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

    // The focus actually handed to `motion` above — none under the hand tool.
    let new_focus = if hand { None } else { under_hit.map(|(target, _)| target) };

    if prev_focus.as_ref() != new_focus.as_ref() {
        // The unlock-restoration warp is queued by `remove_constraint` (smithay
        // announces every deactivation there now) and applied by the drain, rather
        // than inline here — the pointer's mutex is held on this path.
        _loop.state.reevaluate_pointer_constraints(
            &pointer,
            prev_focus.as_ref(),
            new_focus.as_ref(),
        );
    }
}
