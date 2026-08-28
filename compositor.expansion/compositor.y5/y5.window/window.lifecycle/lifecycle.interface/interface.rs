use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::compositor::with_states;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::{
    SurfaceCachedState, ToplevelCachedState, XdgToplevelSurfaceData,
};
use uuid::Uuid;
use compositor_y5_camera_transform_translate::slot;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::{Loop, Transform};
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_y5_window_lifecycle_event::event::WindowLifecycleEvent;
/// Activate `window` on behalf of an external request (e.g. a dock via wlr foreign-toplevel
/// `activate`): ease the camera to frame it (navigator `view`), then raise + activate + give
/// it keyboard focus. Mirrors the keybinding "view window" path.
fn activate_window(_loop: &mut Loop, window: Window) {
    use compositor_y5_navigator_state_base::state::State;
    use compositor_y5_navigator_travel_state::state::{Target, Travel};

    // Cross-world activation: if the window lives on another world, switch to it FIRST
    // (immediately), then frame it. The frame is then a no-animation jump — an eased pan
    // across a just-switched world would be jarring.
    let hosted = _loop.inner.worlds.spawn_target();
    let cross_world = matches!(_loop.inner.world_of_window(&window), Some(w) if w != hosted);
    if cross_world {
        if let Some(w) = _loop.inner.world_of_window(&window) {
            _loop.inner.switch_to_world(w);
        }
    }

    // `fit_absolute = true` adds ZOOM_OUT_TO_FIT so the WHOLE window is framed (zooms out
    // when it's bigger than the screen), matching the Super+Left/Right "view window" feel —
    // not just zoom-in-to-fit. A dock activation should show the whole window.
    let result = compositor_y5_navigator_travel_machine::view::view(_loop, vec![&window], true);
    let travel = Travel {
        position: result.position.map(|target| Target { start: None, target }),
        zoom: result.zoom.map(|target| Target { start: None, target }),
        // Same-world: default eased travel (`None` → the 500ms config default). Cross-world:
        // instant — `0.0`s makes the tick complete on the first frame, so we jump straight to
        // the framing instead of animating a pan on top of the world switch.
        duration: cross_world.then(|| 0.0),
        time_start: None,
    };
    _loop.inner.navigator_mut().set(State::Travel(travel));

    _loop.inner.space_state_mut().state.raise_element(&window, true);
    // Activate the target and DEACTIVATE every other window across all worlds (not just
    // the hosted one). A cross-world activate otherwise leaves the previously-focused
    // window still `activated` in another world; the foreign mirror advertises all worlds
    // (when `all_worlds`), so that stale flag makes the target's re-activation a no-op
    // diff and sfwbar never sees it become focused.
    _loop.inner.set_activated_exclusive(Some(&window));
    let surface = window.toplevel().map(|t| t.wl_surface().clone());
    if let Some(keyboard) = _loop.state.seat.seat.get_keyboard() {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.set_focus(&mut _loop.state, surface, serial);
    }
}

/// Generally all hooks are temporary - they indicate something immediate is being deferred(due to complex ownership.)
/// This hook is temporary because it wires the WireTrait impl and WireObject state.
pub fn hook(_loop: &mut Loop, renderer: &mut GlesRenderer) {
    _apply_toplevel_drag_moves(_loop);

    let process = std::mem::take(
        &mut _loop.inner.window_lifecycle_mut()
            .incoming,
    );
    // generally no-op. _state.inner.window.incoming becomes Vec::default().
    // _loop.inner.window.incoming.clear();

    if process.len() == 0 {
        return;
    }

    for item in process {
        match item {
            compositor_y5_window_lifecycle_event::event::WindowLifecycleEvent::InitialMap(
                window,
            ) => {
                // The window's own birth stamp. `draw.moment` records every later
                // moment by observing the draw, but it is skipped entirely while
                // no bundle is loaded — so the one thing a draw cannot recover
                // afterwards, WHEN this window appeared, is recorded here.
                compositor_y5_window_draw_moment::moment::born(&window);
                _initial_mapped(_loop, window);
            }
            WindowLifecycleEvent::Fullscreen(window, fullscreen) => {
                compositor_y5_window_interface_draw::fullscreen::fullscreen_set(
                    _loop, window, fullscreen,
                );
            }
            WindowLifecycleEvent::Activate(window, _origin) => {
                activate_window(_loop, window);
            }
            WindowLifecycleEvent::DragSettled(surface) => {
                _settled_toplevel_drag(_loop, surface);
            }
            WindowLifecycleEvent::Destroyed(uuid, activation, discard_placeholder) => {
                _destroy(_loop, uuid, renderer, discard_placeholder);

                // CHECK: Token is cleared on surface deletion. if a splash screen uses this token, it will be removed and no longer valid.
                if let Some(activation) = activation {
                    // Clear token
                    let token_cleared = _loop
                        .state
                        .xdg_activation
                        .xdg_activation
                        .remove_token(&activation.token);

                    info!("Token cleared: {:?}", token_cleared)
                }
            }
        }
    }

    // Map/unmap/destroy/fullscreen may have changed the captured window set.
    compositor_y5_graphic_capture_interface::interface::on_window_geometry_changed(_loop);

    _loop.schedule_redraw();
}

/// Apply the positions an `xdg_toplevel_drag_v1` grab queued for the window it
/// is carrying.
///
/// Once per frame, not once per motion. The grab cannot place the window itself
/// (`Dispatch` owns no `Space`) so it queues a world position on every motion,
/// but that position is only ever OBSERVED at render — applying it more often
/// just overwrites values nothing has read, so a higher input rate than the
/// frame rate buys no smoothness. Running here also gives it one defined place
/// in the frame: ahead of the lifecycle queue below, so a drop settling this
/// frame reads the position this frame put down.
///
/// The value is already y5-world — `map_element` stores camera-independent
/// coordinates and the camera is applied at render time — so it goes in
/// unprojected.
fn _apply_toplevel_drag_moves(state: &mut Loop) {
    for (surface, location) in std::mem::take(&mut state.state.toplevel_drag.moves) {
        // Resolved across ALL worlds, and mapped into the one that actually holds
        // it. A drag can outlive the world it started in, and every `space_state`
        // accessor is bound to `spawn_target` — so after a switch the carried
        // window is simply not found, and the move is dropped without a trace:
        // the window freezes where it was and the drop lands at a stale position.
        let Some((world, window)) = state.inner.window_of_surface(&surface) else {
            continue;
        };
        state
            .inner.space_of_mut(world)
            .state
            .map_element(window, location.to_i32_round(), false);
    }
}

/// Sync the placeholder record of a window an `xdg_toplevel_drag_v1` has just
/// finished carrying.
///
/// A window's live placeholder holds the geometry its placeholder will spawn at when
/// the window is eventually closed, and it is normally kept in step by the
/// canvas MOVE system as the window is dragged. A toplevel drag never goes
/// through that — the grab places the window itself — so without this the
/// record still holds the position the window was FIRST MAPPED at, and closing
/// it later drops the placeholder back there. (Nudging the window by hand afterwards
/// made it look correct because that nudge is a canvas move.)
///
/// Driven by `WindowLifecycleEvent::DragSettled` rather than a side channel, so
/// it is ORDERED against the rest of the queue. A tab torn off and dropped in a
/// single frame queues `InitialMap` and this in that order, and the record only
/// exists once the map ahead of it has run — draining a separate list before the
/// queue silently lost exactly that case, which is the one this function is for.
fn _settled_toplevel_drag(state: &mut Loop, surface: WlSurface) {
    // Across all worlds, for the same reason as the moves above.
    let Some((world, window)) = state.inner.window_of_surface(&surface) else {
        return;
    };
    // A uuid is NOT a record: `initialize_surface_data` stamps the uuid and maps
    // the window at (0,0) on the `new_toplevels` drain, while the record only
    // appears once `on_window_map_initial` has handled the `InitialMap`. Ordering
    // makes that the normal case now, but a window whose map never produced a
    // record still lands here, and `interface::set` -> `modify` aborts on a
    // missing record rather than skipping. Same guard as `_destroy`.
    let Some(uuid) = window.uuid() else { return };
    if !state.inner.placeholder().map.contains_key(&uuid) {
        return;
    }
    let Some(location) = state.inner.space_of_mut(world).state.element_location(&window) else {
        return;
    };
    compositor_y5_placeholder_interface_base::interface::set(state, window, None, Some(location));
}

fn _initial_mapped(state: &mut Loop, window: Window) {
    // Resolve the tearing target tag here and nowhere else: this is the one
    // moment the window's process can be introspected off the commit path.
    compositor_y5_graphic_tearing_tag::tag::tag(state, &window);
    // A toplevel that maps while an `xdg_toplevel_drag_v1` is ALREADY carrying it
    // is a torn-off tab, not a new window: the user is holding it, so it belongs
    // under the cursor at its attach offset. Centring it and letting the next
    // pointer motion correct it is exactly the visible flick to mid-screen and
    // back that a tab tear shows.
    //
    // Same arithmetic as the grab (`state.grab/grab.drag.state`): the pointer's
    // location is already y5-world, so the surface-local offset subtracts
    // straight off it with no projection.
    //
    // Resolved BEFORE the restore below, because it decides whether the restore's
    // verdict may be honoured at all.
    let carried = state
        .state
        .toplevel_drag
        .carried_with_offset()
        .filter(|(surface, _)| window.toplevel().is_some_and(|t| t.wl_surface() == surface))
        .and_then(|(_, offset)| {
            let pointer = state.state.seat.seat.get_pointer()?;
            let at = pointer.current_location() - offset.to_f64();
            Some((at.x, at.y))
        });

    // Windows must be registetred at sampler
    // topleevel only
    //
    // Always called, even while carrying: the introspection half (the `NoDisplay`
    // latch, the sampler registration) is owed to every toplevel that maps. Only
    // the RESTORE half is refused for a carried window — a placeholder must not
    // capture a window in flight and yank it to a remembered position mid-gesture.
    let restore_mapped =
        compositor_y5_placeholder_interface_base::interface::on_window_map_initial(
            state,
            window.clone(),
            carried.is_none(),
        );

    if restore_mapped {
        return;
    }

    let geometry = window.geometry();

    // Center on the ACTIVE monitor's camera (the output under the cursor), NOT
    // `camera_mut()`. This hook drains the InitialMap queue from inside the
    // per-output render loop, so `render_output` is pinned to whichever output is
    // being drawn (normally the primary) — `camera_mut()`/`current_output_key()`
    // would resolve THAT output's camera and spawn the window on the wrong monitor.
    // `active_camera()` ignores `render_output` and follows the user's screen.
    let (x, y) = carried.unwrap_or_else(|| {
        let cam = state.inner.active_camera().transform.position();
        (
            cam.x - geometry.size.w as f64 / 2.0,
            cam.y - geometry.size.h as f64 / 2.0,
        )
    });

    let t: Transform = ((x, y), state.size_ctx_all()).into();

    state
        .inner.space_state_mut()
        .state
        .map_element(window.clone(), t.into_storage_point(), false);

    // Dialog/child toplevels (a set `parent`, e.g. nautilus's merge-conflict window) size themselves — leave them `Auto`; lock+grace the rest to the mapped size.
    if window.toplevel().and_then(|t| t.parent()).is_some() {
        slot::set_expected_auto(&window);
    } else {
        slot::set_expected_size(&window, geometry.size);
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| s.size = Some(geometry.size));
            toplevel.send_configure();
        }
        compositor_support_smithay_state_compositor_place::arm_size_propagation(&window, geometry.size);
    }

    // Register the window in the spatial world's draw-order authority
    // (non-destructive; spawn = top of stack).
    if let Some(uuid) = window.uuid() {
        state.inner.register_drawable(uuid, compositor_support_world_order_track_base::base::DrawLayer::CONTENT);
    }

    // // The geometry is unknown. Client may request one but it defaults to a small rectangle in the center.
    // let geometry = window.geometry();
    // let x = state.inner.camera_mut().transform.position().x - (geometry.size.w as f64 / 2.0);
    // let y = state.inner.camera_mut().transform.position().y - (geometry.size.h as f64 / 2.0);
    // let new_location = smithay::utils::Point::from((x as i32, y as i32));
    // Data is known now, so use it to set placeholder data.

    // Attempt to get the window geometry. this is optional. the geometry should update at interface level as well.
    let geometry = state.inner.space_state().state.element_geometry(&window);
    // if let Some(geometry) = geometry {
    //     placeholder.size.0 = geometry.size.w;
    //     placeholder.size.1 = geometry.size.h;
    //     placeholder.position.0 = geometry.loc.x;
    //     placeholder.position.1 = geometry.loc.y;
    // }

    let storage_point = t.into_storage_rect_physical();

    compositor_y5_placeholder_interface_base::interface::set(
        state,
        window,
        // CHECK: Still needs upscale if not already handled in ice.
        geometry.and_then(|w| Some(w.size)),
        Some(Point::new(storage_point.loc.x, storage_point.loc.y)),
    );
}

fn _destroy(state: &mut Loop, uuid: Uuid, renderer: &mut GlesRenderer, discard_placeholder: bool) {
    compositor_y5_placeholder_interface_base::interface::on_window_destroy(state, uuid, renderer, discard_placeholder);
    // invalidate selection.
    compositor_y5_select_interface_base::remove(state, uuid);
    compositor_y5_group_interface_base::interface::window_destroy(state, uuid);
    // DrawOrder GC: drop the window from the draw-order authority.
    state.inner.remove_drawable(uuid);
}

pub struct TransformUpdate {
    pub position: Option<Point<i32, Logical>>,
    pub size: Option<Size<i32, Logical>>,
}

// There are a few places where size is set or modified:
// Initial placement:
//  this is the part of wayland configuration
//  where the client requests a specific size ( or any size )
//  the wayland server decides the size ( at dispatcher's code currently )
//  the wayland server sets the size locally and submits it to the client(which must behave with the decided size)
//
//
// Canvas events
// Grab events - not handled for now. they should be part of WireTrait
// Similarly for movements. location is simplified- the client has no idea. (not sure whether it can request it at all)
// and it is mapped in the place_window call. which should probably call refresh_geometry.
// better yet - to have the initial mapping use place_window and avoid the "WindowPlaced" marker. it is more likely the Window size marker.(eg. post configure size)
//
// This function is used to request a new size/position for a window
pub fn reform(state: &mut Loop, window: Window, transform_update: TransformUpdate) {
    _reform(state, window, transform_update, false);
}

pub fn reform_force(state: &mut Loop, window: Window, transform_update: TransformUpdate) {
    _reform(state, window, transform_update, true);
}

// `finish_resize` moved to `compositor_y5_canvas_system_base` (the release input
// system that uses it) — it is Loop-free (smithay + `slot`), so it lives with its
// only caller rather than in this Loop-coupled crate (which a system can't depend
// on without a cycle via the orchestration focus accessors).

fn _reform(state: &mut Loop, window: Window, transform_update: TransformUpdate, force: bool) {
    if let Some(position) = transform_update.position {
        state
            .inner.space_state_mut()
            .state
            .map_element(window.clone(), position, false);
    }

    if let Some(size) = transform_update.size {
        // Does it expect it to be topleve?
        //yes
        let toplevel = window.toplevel().unwrap_or_else(|| abort!("reform expects toplevels only."));
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
            state.size = Some(size);
        });

        // The compositor's new decided size — the window is enforced at this until the next
        // reform. This is the authority the render/input fit uses.
        slot::set_expected_size(&window, size);

        if force {
            // Interactive resize drag (`reform_force`, from canvas motion): throttle the configure
            // — one client commit per pointer motion is what stutters — and arm the stretch so the
            // window follows the cursor between commits. The final size + settle happen on release
            // (`finish_resize`). `note_resize` returns whether a configure is due now.
            if slot::note_resize(&window, size) {
                toplevel.send_configure();
            }
        } else {
            // One-off resize (navigator maximize, tiling, etc.): send immediately, no throttle and
            // no stretch — there's no drag/release to settle it, so arming the stretch would leave
            // the window stuck stretching and re-sending configures forever.
            toplevel.send_configure();
        }
    }

    if force {
        // let opt = (transform_update.position, transform_update.size);

        // force_window_geometry(&window, opt);
    }

    if let Some(uuid) = window.uuid() {
        compositor_y5_group_interface_base::interface::invalidate_bbox(state, uuid);
    }

    // A window moved/resized — refresh the capture region's tracked bbox +
    // force-render set (event-driven, mirrors the group bbox invalidation).
    compositor_y5_graphic_capture_interface::interface::on_window_geometry_changed(state);

    compositor_y5_placeholder_interface_base::interface::set(
        state,
        window,
        transform_update.size,
        transform_update.position,
    );
}

// fn force_window_geometry(window: &Window, new_geom: Rectangle<i32, Logical>) {
//     let surface = window.wl_surface().unwrap();
//     if let Some(surface) = window.wl_surface() {
//         // It's the set geometry clamped to the bounding box with the full bounding box as the fallback.
//         let details = with_states(&surface, |states| {
//             states
//                 .cached_state
//                 .get::<SurfaceCachedState>()
//                 .current()
//                 .geometry
//                 .and_then(|geo| geo.intersection(bbox))
//         }).unwrap();
//     }
// }

fn force_window_geometry(
    window: &Window,
    new_geom: (Option<Point<i32, Logical>>, Option<Size<i32, Logical>>),
) {
    let Some(surface) = window.wl_surface() else {
        return;
    };

    with_states(&surface, |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let current = cached.current();
        let Some(mut geom) = current.geometry.clone() else {
            return;
        };

        if let Some(position) = new_geom.0 {
            geom.loc = position;
        }

        if let Some(size) = new_geom.1 {
            geom.size = size;
        }

        current.geometry = Some(geom);
    });
}
