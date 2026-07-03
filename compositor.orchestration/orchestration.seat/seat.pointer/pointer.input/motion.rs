use crate::constraint::apply_pointer_constraint;
use crate::native_motion;
use smithay::backend::input::{AbsolutePositionEvent, Event, InputBackend, PointerMotionEvent};
use smithay::utils::{Logical, Physical, Point};
use std::ops::Deref;
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_y5_camera_transform_translate::translate;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;

pub fn absolute<I: InputBackend>(
    event: &<I as InputBackend>::PointerMotionAbsoluteEvent,
    _loop: &mut Loop,
) {
    // Compute the physical screen position (`raw_pos`) and the normalized world
    // point BEFORE the bus so the motion systems receive precisely what the rim
    // computed (the pan delta uses the physical screen, transforms +
    // pointer.motion use the world point).
    // Full-output context maps the normalized absolute event → physical cursor.
    let screen = _loop.size_ctx_all();

    // position_transformed wants Size<_, Logical>. Pass the panel's
    // physical size wrapped as Logical — the values are physical even
    // if the marker says otherwise. We immediately re-tag the result
    // as Physical (which is what it really is).
    let physical_size_as_logical = smithay::utils::Size::<i32, Logical>::from((
        screen.screen_size_physical.0.round() as i32,
        screen.screen_size_physical.1.round() as i32,
    ));
    let raw_pos: Point<f64, Logical> = event.position_transformed(physical_size_as_logical);

    // The numbers are in physical units; re-tag.
    let position_screen = Point::<f64, Physical>::from((raw_pos.x, raw_pos.y));

    // Resolve the pane under the cursor (records it as the `pointer` slot) and map
    // physical → world through THAT pane's camera/region, so input follows the
    // pane the cursor is over, not the keyboard-active pane.
    let ctx = _loop.pointer_context(position_screen);
    let t: Transform = (position_screen, ctx).into();
    let position_normalized = &t.into_storage_point_f64();

    // Separator / floating-pane drag in progress → apply it and move the cursor,
    // but do not route to the canvas/window grab systems.
    if compositor_y5_viewport_interaction_base::interaction::update_separator(_loop.inner.output_views_mut(), position_screen)
        || compositor_y5_viewport_interaction_base::interaction::update_floating(_loop.inner.output_views_mut(), position_screen)
    {
        native_motion::absolute::input_received_normalized::<I>(event, _loop, position_normalized, &raw_pos);
        return;
    }

    {
        // World input bus first (Pass-1): CameraSystem handles the canvas PAN
        // (Hand / position_updating), CanvasSystem the MOVE/SCALE/SELECTBOX
        // transforms. `Pass` falls through to legacy `native_motion` routing.
        let ev = compositor_support_system_input_event_base::base::InputEvent::PointerMotion {
            x: position_normalized.x,
            y: position_normalized.y,
            screen_x: raw_pos.x,
            screen_y: raw_pos.y,
            delta_x: 0.0,
            delta_y: 0.0,
        };
        if compositor_orchestration_input_drive_base::drive::route(_loop, ev)
            == compositor_support_system_input_event_base::base::InputFlow::Consume
        {
            return;
        }
    }

    // let ctx = _loop.size_ctx_all();
    // let output = _loop.inner.space_state().state.outputs().next().unwrap();
    // let logical_geom = _loop.inner.space_state().state.output_geometry(output).unwrap();

    // let position_screen: Point<f64, Logical> = event.position_transformed(logical_geom.size);

    // let t: Transform = (position_screen, ctx).into();
    // let position_normalized = &t.into_storage_point_f64();

    // Extract geometry from compositor space
    // let compositor_output = _loop.inner.space_state().state.outputs().next().unwrap();
    // let compositor_output_geometry = _loop
    //     .state
    //     .space
    //     .state
    //     .output_geometry(compositor_output)
    //     .unwrap();

    // // Get the position of cursor on screen
    // let position_screen = event.position_transformed(compositor_output_geometry.size)
    //     + compositor_output_geometry.loc.to_f64();

    // let ctx = _loop.size_ctx_all();
    // let cursor_phys = Point::<f64, Physical>::from((position_screen.x, position_screen.y));

    // let t: Transform = (cursor_phys, ctx).into();

    // // Extract as raw y5-world, since smithay clients work in world coords.
    // let position_normalized = &t.into_storage_point_f64();

    // // Normalize the cursor position based on camera state
    // // THis is actually screen to world which is also space to world since they share coords.
    // let position_normalized = &translate::space_to_world(
    //     &_loop.inner.camera_mut().transform,
    //     compositor_output_geometry.size.to_f64(),
    //     position_screen,
    //     _loop.inner.space_state().default_scale()
    // );

    // Bus returned Pass (no canvas pan/transform consumed it) — route native
    // client pointer motion.
    native_motion::absolute::input_received_normalized::<I>(
        event,
        _loop,
        position_normalized,
        &raw_pos,
    );
}

/// Try to cross the cursor to an adjacent monitor when it leaves this output's
/// bounds. On success updates `cursor_output`/`cursor_placement` + the per-output
/// current view, and returns the entry point in the NEW output's physical space
/// plus that output's pointer context. `None` when there is no teleport layout, the
/// cursor is still in-bounds, or no placement abuts the crossed edge (clamp).
fn teleport_cross(
    _loop: &mut Loop,
    mx: f64,
    my: f64,
    pw: f64,
    ph: f64,
) -> Option<(Point<f64, Physical>, compositor_y5_camera_transform_translate::transform::Context)> {
    use compositor_orchestration_driver_output_base::base::{Edge, CURSOR_PLACEMENT_MUT, TELEPORT_LAYOUT};
    // Teleport is suppressed while ANY system holds the suppression lock (a refcount);
    // a canvas pan is the built-in client (it pins the cursor to its output for the pan's
    // duration). This path knows nothing about pan specifically — just the lock.
    if _loop.inner.teleport_suppressed() || _loop.inner.kernel.get(&TELEPORT_LAYOUT).is_empty() {
        return None;
    }
    // Which edge did the cursor cross, and where along it (proportionally 0..1)?
    let (edge, frac) = if mx < 0.0 {
        (Edge::Left, (my / ph) as f32)
    } else if mx > pw {
        (Edge::Right, (my / ph) as f32)
    } else if my < 0.0 {
        (Edge::Top, (mx / pw) as f32)
    } else if my > ph {
        (Edge::Bottom, (mx / pw) as f32)
    } else {
        return None; // still inside the output → not a crossing
    };
    let from_id = current_placement(_loop)?;
    let n = _loop.inner.kernel.get(&TELEPORT_LAYOUT).neighbor(from_id, edge, frac)?;
    // Adopt the entered monitor + zone. The teleport layout and `output_key` share
    // the same EDID identity, so the placement key IS the output key. Point the
    // per-output view state at it so the input systems (pan/zoom on this monitor)
    // operate on THIS monitor's own camera.
    _loop.inner.cursor_output = Some(n.key.clone());
    _loop.inner.output_views_mut().set_current(&n.key);
    *_loop.inner.kernel.get_mut(&CURSOR_PLACEMENT_MUT) = Some(n.id);
    // Entry point on the new output (opposite the crossed edge, 1px inset).
    let (npw, nph) = _loop.size_ctx_all().screen_size_physical;
    const INSET: f64 = 1.0;
    let ef = n.entry_frac as f64;
    let entry: Point<f64, Physical> = match n.entry_edge {
        Edge::Left => Point::from((INSET, ef * nph)),
        Edge::Right => Point::from((npw - INSET, ef * nph)),
        Edge::Top => Point::from((ef * npw, INSET)),
        Edge::Bottom => Point::from((ef * npw, nph - INSET)),
    };
    let new_ctx = _loop.pointer_context(entry);
    Some((entry, new_ctx))
}

/// The placement the cursor currently occupies, seeded on first use to the current
/// output's first placement (else the first placement overall).
fn current_placement(_loop: &mut Loop) -> Option<u64> {
    use compositor_orchestration_driver_output_base::base::{CURSOR_PLACEMENT, CURSOR_PLACEMENT_MUT, TELEPORT_LAYOUT};
    if let Some(id) = *_loop.inner.kernel.get(&CURSOR_PLACEMENT) {
        if _loop.inner.kernel.get(&TELEPORT_LAYOUT).get(id).is_some() {
            return Some(id);
        }
    }
    let key = _loop.inner.current_output_key();
    let id = {
        let t = _loop.inner.kernel.get(&TELEPORT_LAYOUT);
        t.first_of(&key).or_else(|| t.placements.first()).map(|p| p.id)?
    };
    *_loop.inner.kernel.get_mut(&CURSOR_PLACEMENT_MUT) = Some(id);
    Some(id)
}

pub fn relative<I: InputBackend>(
    event: &<I as InputBackend>::PointerMotionEvent,
    _loop: &mut Loop,
) {
    // Full-output context for the physical-accumulator clamp bounds.
    let screen = _loop.size_ctx_all();

    let dt = event.delta();
    let dt_unaccelerated = event.delta_unaccel();

    // Live cursor-speed multiplier (settings window). Read before the
    // `pointer_mut()` borrows below so the borrows stay disjoint. Applied ONLY to
    // the on-screen cursor accumulator — NOT to `dt`/`dt_unaccelerated`, which are
    // forwarded raw to clients via the relative-pointer protocol (games apply
    // their own acceleration and expect unscaled hardware deltas).
    let sensitivity = _loop.inner.preference.cursor_sensitivity;

    // Snapshot previous position in both spaces.
    let previous_phys = _loop.inner.pointer_mut().motion.clone();
    // Pane under the cursor (records the `pointer` slot); map world through it.
    let ctx = _loop.pointer_context(Point::<f64, Physical>::from((previous_phys.x, previous_phys.y)));
    let previous_world: Point<f64, Logical> = {
        let pt = Point::<f64, Physical>::from((previous_phys.x, previous_phys.y));
        let t: Transform = (pt, ctx).into();
        t.into_storage_point_f64()
    };

    // Accumulate the physical delta, scaled by the live cursor sensitivity.
    _loop.inner.pointer_mut().motion.x += dt.x * sensitivity;
    _loop.inner.pointer_mut().motion.y += dt.y * sensitivity;

    // Candidate in world space.
    let candidate_world: Point<f64, Logical> = {
        let pt = Point::<f64, Physical>::from((
            _loop.inner.pointer_mut().motion.x,
            _loop.inner.pointer_mut().motion.y,
        ));
        let t: Transform = (pt, ctx).into();
        t.into_storage_point_f64()
    };

    // CHECK: Constraint is not applied for absolute events. Relevant for
    // tablets and winit testing; otherwise redundant.
    let (constrained_world, was_constrained) =
        apply_pointer_constraint(_loop, previous_world, candidate_world);

    // Reconcile final world position and update the physical accumulator
    // so future deltas accumulate from the right place.
    let final_world: Point<f64, Logical> = if was_constrained {
        // Reverse-project constrained world back to physical, write to accumulator.
        let final_phys: Point<f64, Physical> = {
            let t: Transform = (constrained_world, ctx).into();
            t.into()
        };
        _loop.inner.pointer_mut().motion.x = final_phys.x;
        _loop.inner.pointer_mut().motion.y = final_phys.y;
        constrained_world
    } else {
        // No constraint. If the cursor left this output's bounds and a teleport
        // layout places an adjacent monitor across that edge, cross to it; else
        // clamp to this output's bounds (the edge is a layout boundary).
        let (pw, ph) = screen.screen_size_physical;
        let mx = _loop.inner.pointer_mut().motion.x;
        let my = _loop.inner.pointer_mut().motion.y;
        match teleport_cross(_loop, mx, my, pw, ph) {
            Some((entry, new_ctx)) => {
                _loop.inner.pointer_mut().motion.x = entry.x;
                _loop.inner.pointer_mut().motion.y = entry.y;
                let t: Transform = (entry, new_ctx).into();
                t.into_storage_point_f64()
            }
            None => {
                _loop.inner.pointer_mut().motion.x = mx.clamp(0.0, pw);
                _loop.inner.pointer_mut().motion.y = my.clamp(0.0, ph);
                let pt = Point::<f64, Physical>::from((
                    _loop.inner.pointer_mut().motion.x,
                    _loop.inner.pointer_mut().motion.y,
                ));
                let t: Transform = (pt, ctx).into();
                t.into_storage_point_f64()
            }
        }
    };

    // Keep the per-output view state's `current` on the output the cursor is on, so
    // the input systems (pan/zoom, hit-test) operate on THIS monitor's own viewport
    // — not the last-rendered one. `cursor_output` is maintained by teleport
    // crossings and persists between them; initialize it to the primary on first use.
    if _loop.inner.cursor_output.is_none() {
        _loop.inner.cursor_output =
            Some(compositor_orchestration_core_state_base::state::output_key(_loop.inner.current_output()));
    }
    if let Some(co) = _loop.inner.cursor_output.clone() {
        _loop.inner.output_views_mut().set_current(&co);
    }

    let position_screen = _loop.inner.pointer_mut().motion;
    let position_normalized = final_world;

    // Apply motion only if this is equals false
    // !was_constrained || final_world != previous_world a
    
    let was_constrained_locked = !(!was_constrained || final_world != previous_world);

    // Previous code start(working pre motion relative and constraint impl.)

    // Accumulate cursor in physical pixels (libinput's natural unit).
    // let dt = event.delta();
    // let dt_unaccelerated = event.delta_unaccel();
    // let time = event.time_msec();

    // let previous_location = _loop.inner.pointer_mut().motion.clone();

    // _loop.inner.pointer_mut().motion.x += dt.x;
    // _loop.inner.pointer_mut().motion.y += dt.y;

    // // Clamp to physical panel bounds.
    // let (pw, ph) = ctx.screen_size_physical;

    // // CHECK: Constraint is not applied for absolute events. This is especially relevant for tablets and testing within winit. but otherwise redaundant.
    // let (constrained, was_constrained) =
    //     apply_pointer_constraint(_loop, previous_location, _loop.inner.pointer_mut().motion);

    // let new_location = if was_constrained {
    //     constrained
    // } else {
    //     Point::new(
    //         _loop.inner.pointer_mut().motion.x.clamp(0.0, pw),
    //         _loop.inner.pointer_mut().motion.y.clamp(0.0, ph),
    //     )
    // };

    // _loop.inner.pointer_mut().motion = new_location;

    // // Additionally clamp by constraint.

    // // Build a Transform from the physical cursor position. Transform
    // // reverse-projects through scale + camera, storing as y5-world.
    // let cursor_phys =
    //     Point::<f64, Physical>::from((_loop.inner.pointer_mut().motion.x, _loop.inner.pointer_mut().motion.y));

    // let t: Transform = (cursor_phys, ctx).into();

    // let position_screen = _loop.inner.pointer_mut().motion; // < -- NOTE: This variable is being used for calculating deltas for panning
    // // Extract as raw y5-world, since smithay clients work in world coords.
    // let position_normalized = t.into_storage_point_f64(); // < -- NOTE: This variable is currently the variable sent to pointer motion

    // Previous code end //

    // 1. Extract geometry from compositor space (Exactly like your absolute arm)
    // let compositor_output = _loop.inner.space_state().state.outputs().next().unwrap();
    // let compositor_output_geometry = _loop
    //     .state
    //     .space
    //     .state
    //     .output_geometry(compositor_output)
    //     .unwrap();

    // let compositor_output_geometry_physical = _loop.inner.space_state().default_physical_precise();
    // let compositor_output_geometry_logical = _loop.inner.space_state().default_logical();

    // let compositor_output_geometry = _loop.inner.space_state().state.outputs().next().unwrap();
    // let compositor_output_geometry_logical = _loop.inner.space_state().state.output_geometry(compositor_output_geometry).unwrap();

    // 2. Calculate the new screen position using the hardware delta
    // Note: You must add `pub pointer_location: smithay::utils::Point<f64, Logical>`
    // to your `Loop` struct to accumulate this movement!
    // let dt = event.delta();
    // let mut position_screen = _loop.inner.pointer_mut().motion + dt;

    // 3. Clamp to screen bounds so the cursor cannot leave the monitor
    // let min_x = compositor_output_geometry_logical.loc.x as f64;
    // let max_x = (compositor_output_geometry_logical.loc.x + compositor_output_geometry_logical.size.w) as f64;
    // let min_y = compositor_output_geometry_logical.loc.y as f64;
    // let max_y = (compositor_output_geometry_logical.loc.y + compositor_output_geometry_logical.size.h) as f64;

    // It is better that this works logically like now.
    // But, position_normalized performs space_to_world which is in turn:
    // 1. does space_to_world
    // position_screen.x = position_screen.x.clamp(min_x, max_x);
    // position_screen.y = position_screen.y.clamp(min_y, max_y);

    // Save the clamped position for the next frame's relative calculation
    // _loop.inner.pointer_mut().motion = position_screen;

    // 5. Normalize the cursor position based on camera state
    // let position_normalized = translate::space_to_world(
    //     &_loop.inner.camera_mut().transform,
    //     compositor_output_geometry_logical.size.to_f64(),
    //     position_screen,
    //     _loop.inner.space_state().default_scale()
    // );
    // 4. Notify canvas of input to handle camera movement

    // Separator / floating drag in progress → apply + move cursor, skip canvas.
    // `position_screen` carries physical values under a Logical marker; re-tag.
    let cursor_phys = Point::<f64, Physical>::from((position_screen.x, position_screen.y));
    if compositor_y5_viewport_interaction_base::interaction::update_separator(_loop.inner.output_views_mut(), cursor_phys)
        || compositor_y5_viewport_interaction_base::interaction::update_floating(_loop.inner.output_views_mut(), cursor_phys)
    {
        native_motion::relative::input_received_normalized::<I>(
            event,
            _loop,
            position_normalized,
            &position_screen,
            (dt, dt_unaccelerated),
            was_constrained_locked,
        );
        return;
    }

    // World input bus first (Pass-1), AFTER position_normalized + the
    // pointer-constraint reconciliation: the systems receive the post-constraint
    // world point (`position_normalized`) and the physical accumulator
    // (`position_screen`) — exactly what the rim's canvas motion handler used.
    {
        let ev = compositor_support_system_input_event_base::base::InputEvent::PointerMotion {
            x: position_normalized.x,
            y: position_normalized.y,
            screen_x: position_screen.x,
            screen_y: position_screen.y,
            delta_x: dt.x,
            delta_y: dt.y,
        };
        if compositor_orchestration_input_drive_base::drive::route(_loop, ev)
            == compositor_support_system_input_event_base::base::InputFlow::Consume
        {
            return;
        }
    }

    let position_normalized = position_normalized;
    // 6. Dispatch normalize event to your native pointer handler
    // (Using the equivalent handler for PointerMotionEvent as you mentioned)
    native_motion::relative::input_received_normalized::<I>(
        event,
        _loop,
        position_normalized,
        &position_screen,
        (dt, dt_unaccelerated),
        was_constrained_locked,
    );
}
