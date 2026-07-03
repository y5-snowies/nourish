use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
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
/// Generally all hooks are temporary - they indicate something immediate is being deferred(due to complex ownership.)
/// This hook is temporary because it wires the WireTrait impl and WireObject state.
pub fn hook(_loop: &mut Loop, renderer: &mut GlesRenderer) {
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
                _initial_mapped(_loop, window);
            }
            WindowLifecycleEvent::Fullscreen(window, fullscreen) => {
                compositor_y5_window_interface_draw::fullscreen::fullscreen_set(
                    _loop, window, fullscreen,
                );
            }
            WindowLifecycleEvent::Destroyed(uuid, activation) => {
                _destroy(_loop, uuid, renderer);

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

// Temp, should be an immediate invokation rather than through hook
fn _initial_mapped(state: &mut Loop, window: Window) {
    // Windows must be registetred at sampler
    // topleevel only
    let restore_mapped =
        compositor_y5_placeholder_interface_base::interface::on_window_map_initial(
            state,
            window.clone(),
        );

    if restore_mapped {
        return;
    }

    // Center on the ACTIVE monitor's camera (the output under the cursor), NOT
    // `camera_mut()`. This hook drains the InitialMap queue from inside the
    // per-output render loop, so `render_output` is pinned to whichever output is
    // being drawn (normally the primary) — `camera_mut()`/`current_output_key()`
    // would resolve THAT output's camera and spawn the window on the wrong monitor.
    // `active_camera()` ignores `render_output` and follows the user's screen.
    let geometry = window.geometry();
    let cam = state.inner.active_camera().transform.position();
    let x = cam.x - geometry.size.w as f64 / 2.0;
    let y = cam.y - geometry.size.h as f64 / 2.0;

    let t: Transform = ((x, y), state.size_ctx_all()).into();

    state
        .inner.space_state_mut()
        .state
        .map_element(window.clone(), t.into_storage_point(), false);

    // The compositor decides the window's size, but at map it has made no explicit decision yet:
    // put the window in `Auto` so the slot follows the client's committed `geometry()` while it
    // settles. A client whose first mapped buffer precedes its final `window_geometry` (CSD shadow
    // excluded a frame later, a saved size applied on the next commit) then fills its frame instead
    // of being covered & cropped at the stale first-frame size. The slot freezes (enforced) the
    // moment the compositor reforms/tiles/fullscreens it (`set_expected_size`). A 0x0 geometry is
    // still treated as "no size yet" by `expected_size` → the renderer falls back to native.
    slot::set_expected_auto(&window);

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

fn _destroy(state: &mut Loop, uuid: Uuid, renderer: &mut GlesRenderer) {
    compositor_y5_placeholder_interface_base::interface::on_window_destroy(state, uuid, renderer);
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
