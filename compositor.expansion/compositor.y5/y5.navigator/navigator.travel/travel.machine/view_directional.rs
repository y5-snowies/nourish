use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size};
use std::collections::HashMap;
use compositor_support_action_camera_find_base::find::{
    Direction, WindowEntry, WindowFinderFlags, WindowId,
};
use compositor_support_action_camera_fit_element::element::{
    CameraPlacementFlags, PlacementResult, compute_placement,
};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_draw::visible::DrawWindow;
use compositor_y5_window_interface_record::window::LoopWindow;

struct Modifier;
impl Modifier {
    pub const None: u32 = 1 << 0;
    pub const Fit: u32 = 1 << 1; // <-- Whether to fit
    pub const Move: u32 = 1 << 2; // <-- Whether to move
    pub const Focused: u32 = 1 << 3; // <-- To focused windows
    pub const Selected: u32 = 1 << 4; // <-- Former, or fallback to selected primary
    pub const Visible: u32 = 1 << 5; // <-- Former, or fallback to visible
    pub const Multiple: u32 = 1 << 6; // <-- For selected, all selected. for visible, all visible
}

pub struct Results {
    pub position: Option<(f64, f64)>,
    pub zoom: Option<f64>,
}

// Issue 1:
// A modifier is needed to determine whether to Fit, with Fit, movement should occur only for remaining points. ( eg. it may move only on X instead of trying to center )
// WIth fit- it is an exact fit, it wont go to center

pub fn view_directional(state: &mut Loop, direction: Direction, alternative: bool) -> Results {
    info!(
        "view_directional was called. camera.zoom={:?}, camera.position={:?}",
        state.inner.camera().transform.zoom(),
        state.inner.camera().transform.position()
    );

    // The ACTIVE output (the one the camera being moved belongs to), not the
    // primary/first — otherwise on multi-monitor the directional view is computed
    // against the wrong screen size. Matches `camera()`/`size_ctx_all()`.
    let output = state.inner.current_output();
    let output_geom_i32 = state
        .inner.space_state()
        .state
        .output_geometry(output)
        .unwrap_or_else(|| abort!("output has geometry"));
    let screen_size: Size<f64, Logical> = output_geom_i32.size.to_f64();

    // ---- 2. Outputs vec for finder, in WORLD coords ------------------------
    // World == Smithay space. Camera (position, zoom) maps world → screen at
    // render time via world_to_space. The viewport in world coords is the
    // inverse: a rect of size screen/zoom, centered on camera.position.
    let viewport_world = Rectangle::new(
        Point::from((
            state.inner.camera_mut().transform.position().x
                - screen_size.w / (2.0 * state.inner.camera_mut().transform.zoom()),
            state.inner.camera_mut().transform.position().y
                - screen_size.h / (2.0 * state.inner.camera_mut().transform.zoom()),
        )),
        Size::from((
            screen_size.w / state.inner.camera_mut().transform.zoom(),
            screen_size.h / state.inner.camera_mut().transform.zoom(),
        )),
    );
    let outputs = vec![viewport_world];

    // // ---- 1. Output and screen size -----------------------------------------
    // // Single output. Screen size is in space coords (top-left anchored).
    // let output = state.compositor.space.outputs().next()
    //     .expect("at least one output");
    // let output_geom_i32 = state.compositor.space
    //     .output_geometry(output)
    //     .expect("output has geometry");
    // let screen_size: Size<f64, Logical> = output_geom_i32.size.to_f64();
    //
    // // ---- 2. Outputs vec for finder, in WORLD coords ------------------------
    // // The viewport (full screen) transformed into world coords. Both screen
    // // corners go through space_to_world; the resulting rect is what the user
    // // is currently "looking at" in world space.
    // let viewport_tl = space_to_world(
    //     &state.camera, screen_size, Point::from((0.0, 0.0)),
    // );
    // let viewport_br = space_to_world(
    //     &state.camera, screen_size, Point::from((screen_size.w, screen_size.h)),
    // );
    // let viewport_world = Rectangle::new(
    //     viewport_tl,
    //     Size::from((viewport_br.x - viewport_tl.x, viewport_br.y - viewport_tl.y)),
    // );
    // let outputs = vec![viewport_world];

    info!("Determined output size viewport_world: {:?}", outputs);

    // ---- 3. Windows + id-to-window map -------------------------------------
    // Smithay window positions are in space coords. Transform each rect into
    // world coords by running BOTH corners through space_to_world (zoom may
    // scale the size, so we can't just translate the loc).
    let elements: Vec<_> = state
        .inner.space_state()
        .state
        .elements()
        .filter(|w| w.toplevel().is_some()) // toplevels only, no popups
        .collect();

    let mut id_map: HashMap<WindowId, smithay::desktop::Window> =
        HashMap::with_capacity(elements.len());
    let mut windows: Vec<WindowEntry> = Vec::with_capacity(elements.len());

    // This doesnt use window location appearently since space location must be used?
    for w in elements {
        let rect_space = state
            .inner.space_state()
            .state
            .element_geometry(w)
            .unwrap_or_else(|| abort!("element has geometry"))
            .to_f64();

        // let tl_world = space_to_world(
        //     &state.camera, screen_size, rect_space.loc,
        // );
        // let br_world = space_to_world(
        //     &state.camera, screen_size,
        //     Point::from((
        //         rect_space.loc.x + rect_space.size.w,
        //         rect_space.loc.y + rect_space.size.h,
        //     )),
        // );
        // let rect_world = Rectangle::new(
        //     tl_world,
        //     Size::from((br_world.x - tl_world.x, br_world.y - tl_world.y)),
        // );

        let id = w.uuid();
        if id.is_none() {
            continue;
        }
        if !w.visible(state) {
            continue;
        }

        let id = id.unwrap();

        info!("Added window with rect_space: {:?}", rect_space);
        // window_id(w);
        id_map.insert(id, w.clone());
        windows.push(WindowEntry {
            id,
            rect: rect_space,
        });
    }

    // ---- 4. Focused window id ----------------------------------------------
    // No element_for_surface in current smithay — find the toplevel whose
    // wl_surface matches the keyboard focus surface.
    let focused: Option<WindowId> = state
        .state
        .seat
        .seat
        .get_keyboard()
        .and_then(|kb| kb.current_focus())
        .and_then(|focus_surface| {
            state.inner.space_state().state.elements().find(|w| {
                w.toplevel()
                    .and_then(|t| Some(t.wl_surface()))
                    .map(|s| s == &focus_surface)
                    .unwrap_or(false)
            })
        })
        .map_or(None, |w| w.uuid());

    info!("focused: {:?}", focused);

    // Also, previous, about the origin changes, our conversation has been cut off,  so I am not sure you have finished it all.
    // Also, we dont have a flag to indicate whether we attempt to navigate per our selected window, like if going up, then to fit the top-right of the screen with the top-right of the bounding box. this should also be a flag.
    // ---- 5. Run the finder -------------------------------------------------
    let flags = WindowFinderFlags::ORIGIN_FOCUSED_VISIBLE
        | WindowFinderFlags::ORIGIN_VISIBLE
        | WindowFinderFlags::RAYCAST_BASE
        | WindowFinderFlags::ORIGIN_MOST_CENTERED // <-- added these 2 flags now, but they should never matter
        | WindowFinderFlags::ORIGIN_MOST_VISIBLE_AREA // <-- added these 2 flags now, but they should never matter
        | WindowFinderFlags::RAYCAST_CYCLING_BASE
        | WindowFinderFlags::RAYCAST_SCREEN_LOW
        | WindowFinderFlags::RAYCAST_SCREEN_HIGH
        | WindowFinderFlags::RAYCAST_CYCLING_SCREEN
        | WindowFinderFlags::RAYCAST_SCREEN_EXTRA
        | WindowFinderFlags::RAYCAST_CYCLING_SCREEN_EXTRA
        | WindowFinderFlags::RAYCAST_CYCLING_ALL
        | WindowFinderFlags::SORT_AXIS_ORIGIN_X
        | WindowFinderFlags::SORT_AXIS_ORIGIN_Y;

    let flags = if (alternative) {
        flags | WindowFinderFlags::RAYCAST_STRETCH
    } else {
        flags
    };

    // Gets results correctly
    // view_directional was called. camera.zoom=0.9569274946159368, camera.position=Point<smithay::utils::geometry::Logical> { x: -273.5, y: -304.5 }
    // Determined output size viewport_world: [Rectangle<smithay::utils::geometry::Logical> { x: -1604.3218304576144, y: -1001.0, width: 2661.643660915229, height: 1393.0 }]
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: 821.0, y: -440.0, width: 800.0, height: 600.0 }
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: 2523.0, y: -448.0, width: 800.0, height: 600.0 }
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: -2148.0, y: -557.0, width: 923.0, height: 297.0 }
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: -789.0, y: -1001.0, width: 1031.0, height: 1393.0 }

    // Removed the big window- doesnt get results.( and unrelated, but panned a bit)
    // view_directional was called. camera.zoom=0.9422035421508697, camera.position=Point<smithay::utils::geometry::Logical> { x: 487.4591527198561, y: -277.13119117589156 }
    // Determined output size viewport_world: [Rectangle<smithay::utils::geometry::Logical> { x: -864.1596249943989, y: -984.5154984758923, width: 2703.23755542851, height: 1414.7686146000015 }]
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: 2523.0, y: -448.0, width: 800.0, height: 600.0 }
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: -2148.0, y: -557.0, width: 923.0, height: 297.0 }
    // Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: 821.0, y: -440.0, width: 800.0, height: 600.0 }

    info!("Calling find with the determined output size and the windows");
    let results = compositor_support_action_camera_find_base::find::find(
        flags, direction, &windows, &outputs, focused,
    );
    info!("REsults: {:?}", results);

    if let Some(res) = results.first() {
        let (result_ids, bbox) = (res.ids.clone(), res.bbox);

        // ---- 6. Apply focus ----------------------------------------------------
        if let Some(target_id) = result_ids.first() {
            if let Some(target_window) = id_map.get(target_id) {
                // Get the surface to focus and set keyboard focus on it.
                // Replace `serial` with your event serial source.
                if let Some(surface) = target_window.toplevel().and_then(|s| Some(s.wl_surface())) {
                    let kb = state.state.seat.seat.get_keyboard().unwrap();

                    // Window-specific: raise, activate, configure.
                    // CHECK : This can happen after position changes.
                    state.inner.space_state_mut().state.raise_element(target_window, true);
                    // Visual top-level z is the draw-order authority, not the smithay
                    // space order — raise it too (mirrors native_press/press.rs).
                    if let Some(uuid) = target_window.uuid() {
                        state.inner.raise_drawable(uuid);
                    }
                    for w in state.inner.space_state().state.elements() {
                        w.set_activated(w == target_window);
                        if let Some(toplevel) = w.toplevel() {
                            toplevel.send_pending_configure();
                        }
                    }

                    let serial = SERIAL_COUNTER.next_serial();
                    kb.set_focus(&mut state.state, Some(surface.clone()), serial);
                }
            }
        }

        // bbox is `Option<Rectangle<f64, Logical>>` in WORLD coords. Use it for
        // animation, viewport adjustment, focus indicator, etc. Example:

        /// This is a very nice placement strategy.
        /// Issue.1. Perp not adjusted which cause offscreen issues. it is more "convinient" and navigation again on the other direction easily refresh that.
        /// Issue.2. Zoom is consolidated throughout the navigation session. It may be more convinient. especially when searching views.
        /// A single large view, however, will break this consolidation and it may become harder to navigate.
        let set_1 = CameraPlacementFlags::PAN_CENTER
            | CameraPlacementFlags::PAN_FIT          // implicit split: CENTER on primary, FIT on perp
            | CameraPlacementFlags::ZOOM_OUT_TO_FIT  // zoom out only if needed
            | CameraPlacementFlags::PAD_DEFAULT;

        /// More aggresive on the perp. really nice. secondary action can cause a fit easily if needed.
        /// Perp is nice. there is not much inconviniece. may feel less "professional" in the term that it moves in 2 directions
        /// Issue.2 remains
        let set_2 = CameraPlacementFlags::PAN_CENTER
            | CameraPlacementFlags::ZOOM_OUT_TO_FIT  // zoom out only if needed
            | CameraPlacementFlags::PAD_DEFAULT;

        /// Also zooms in. Should solve issue 2 but breaks consolidatio ncompletely. no more "overview" navigation easily.
        ///
        /// This works amazing but lacks the bounding box selection algorithm.
        /// (THE FIX CURRENTLY PENDING) It must check the result of base and the result of screen and the result of screen extra. this is the ultimate fix. ( flag for each ) Probably should prefer not combining on the perp axis. ( there is some issue, but example if some window are outside the selected bounding box, but after navigation they'll appear cutoff, then it should probably reconsolidate based on all windows that are available to prevent any cutoff. this can be a no-post-cutoff flag). IT should probably be separated into bbox combiner function.
        /// (AN ADDITIONAL FIX PENDING FOR FINDER) the margin error allowance.
        /// After bounding box selection algorithm, this is the best version.
        let set_3 = CameraPlacementFlags::PAN_CENTER
            | CameraPlacementFlags::ZOOM_IN_TO_FIT
            | CameraPlacementFlags::ZOOM_OUT_TO_FIT
            | CameraPlacementFlags::PAD_DEFAULT;

        // THis is the best when resolving all issues, the only thing: it enforce consolidation, requiring secondary action.
        // ( Still, the previous issues must resolve )
        let set_4 = CameraPlacementFlags::PAN_CENTER
            | CameraPlacementFlags::ZOOM_IN_TO_FIT
            | CameraPlacementFlags::ZOOM_OUT_TO_FIT
            | CameraPlacementFlags::ZOOM_GOAL_MIN_CHANGE
            | CameraPlacementFlags::ZOOM_GOAL_FILL_VIEWPORT
            | CameraPlacementFlags::ZOOM_GOAL_NO_CROP
            | CameraPlacementFlags::PAN_GOAL_MIN_MOVEMENT
            | CameraPlacementFlags::PAN_GOAL_MAX_VISIBILITY
            | CameraPlacementFlags::PAN_GOAL_NO_CUTOFF
            | CameraPlacementFlags::PAN_GOAL_NO_OVERSHOOT
            | CameraPlacementFlags::PAN_DOMINANCE
            | CameraPlacementFlags::ZOOM_DOMINANCE
            | CameraPlacementFlags::PAD_DEFAULT;

        // Same as above but doesnt force consolidation.
        // Explanation on what is consolidation:
        // 1. If I am navigating alot, it probably means i am trying to find something.
        //    with ZOOM_GOAL_MIN_CHANGE, many navigations usually results in a consolidation of the zoom level that fits to all views.
        //    without it, the navigation mostly primary purpose, to act as a 1 key next to best comfortable view, is working great.
        // Easily fixed by adding alt to the shortcut(alt- alternative) when alt is clicked, the zoom goal min change can be enabled, and perhaps other flags, such as stretch
        // Shortcuts are already set. not sure i should only select the set though
        // One thing to note for this one: It seems padding are not enough. i get windows that are extend right to the corner of the screen which makes it so that even window decorations are not visible.
        //
        // another thing: i think we used the window width as the first band pass, it should probably check and use screen width if its lower ( preferring extension ) and a constant low value/50% of screen or some sorts, as the first pass. ( for as long as it is lower than the focused window.

        //
        // Other considerations: the band passes should probably be part of result. these passes, especially in consolidation(alt), can help prefer using the +screen width length instead of the +window width length in cases where it would yield a nice bounding box.

        // As a final conclusion, the most important next step, is to either follow up all notes above and plan ahead for the bound combining function, or simply support the alt button to prefer using screen / screen extra passes + the change of band length where it results in multiple band passes.
        // In both case, the notes must be summarized before we choose a plan ahead.
        // Note: in any case, no change will affect current behaviour. current configuration for finder and fitter works really good but just with those small quirks and these quirks fixes should appear as incremental flags.

        // Q: Where should pass-selection / bbox-combination live?
        //
        // A: We can define nice movements: An exactly full screen upward is a solid and convinient movement for example. An even nicer movement is where you have 2 big windows side by side filling the screen. now there are 2 small windows above side by side (they are offscreen currently), even though first() returns 1 only from the 2 window, the transition could fit the 2 windows side by side as if they were the same big windows at the start.
        //
        // A nice movement is the same side-by-side windows are on screen on both rows but when only the first window is mostly visible and the window from above has been yielded by the base pass or close to base pass then it can go specifically to isolate that window from above.
        //
        // Q: How should pass selection be expressed?
        //
        // A: Explained in 1.
        //
        //
        //
        // Q: How are selected bboxes combined into one?
        //
        // A: Union (axis-aligned bounding rect of selected bboxes) — the only sensible default
        //
        //
        //
        //
        //
        // BBOx "inclusion" flags are helpful in any case as starting flags for our flag composition.
        //
        //
        //
        // II also had some time during our cut off to think about the plans:
        //
        //
        //
        //
        //
        // ```
        //
        //
        //
        // ```
        let set_5 = CameraPlacementFlags::PAN_CENTER
            | CameraPlacementFlags::ZOOM_IN_TO_FIT
            | CameraPlacementFlags::ZOOM_OUT_TO_FIT
            // | CameraPlacementFlags::ZOOM_GOAL_MIN_CHANGE
            | CameraPlacementFlags::ZOOM_GOAL_FILL_VIEWPORT
            | CameraPlacementFlags::ZOOM_GOAL_NO_CROP
            | CameraPlacementFlags::PAN_GOAL_MIN_MOVEMENT
            | CameraPlacementFlags::PAN_GOAL_MAX_VISIBILITY
            | CameraPlacementFlags::PAN_GOAL_NO_CUTOFF
            | CameraPlacementFlags::PAN_GOAL_NO_OVERSHOOT
            | CameraPlacementFlags::PAN_DOMINANCE
            | CameraPlacementFlags::ZOOM_DOMINANCE
            | CameraPlacementFlags::PAD_DEFAULT;

        // On alternative, the stretch was also set as input in finder.
        let set = if alternative { set_4 } else { set_5 };

        let placement = compute_placement(
            set,
            bbox,
            PlacementResult {
                position: *state.inner.camera_mut().transform.position(),
                zoom: *state.inner.camera_mut().transform.zoom(),
            },
            screen_size,
            // viewport_world.size, // <-- probably more accurate
            direction,
        );
        info!("Results are in: {:?}", placement);
        return Results {
            position: Some((placement.position.x, placement.position.y)),
            zoom: Some(placement.zoom),
        };

        // let placement = compute_placement(placement_flags, bbox, current, screen_size, dir);
        //
        //
        // if let Some(bbox) = bbox {
        //     let bbox_center_x = bbox.loc.x + bbox.size.w / 2.0;
        //     let bbox_center_y = bbox.loc.y + bbox.size.h / 2.0;
        //
        //     tracing::info!("Results found: target Position is: {:?}", (bbox_center_x, bbox_center_y));
        //     return Results {
        //         position: Some((bbox_center_x, bbox_center_y)),
        //         zoom: None,
        //     };
        // }
    }

    // window_fit::compute_placement()

    return Results {
        position: None,
        zoom: None,
    };
}

// Stable u64 identifier for a Smithay Window.
// Uses the Arc pointer address — stable across calls for the same window.
// Distinct from window_finder's SYNTHETIC_ORIGIN_ID (u64::MAX) because no
// real Arc allocation can sit at that address.
// fn window_id(window: &smithay::desktop::Window) -> WindowId {
//     // smithay::desktop::Window wraps an Arc internally; the cleanest stable
//     // id is the toplevel surface's WlSurface id, but Arc<inner> address also
//     // works. Pick whichever your codebase uses elsewhere for window identity.
//     use smithay::reexports::wayland_server::Resource;
//     window.toplevel()
//         .and_then(|t| Some(t.wl_surface()))
//         .map(|s| s.id().protocol_id() as u64)
//         .unwrap_or(0)
// }

// view_directional was called. camera.zoom=0.7513148009015774, camera.position=Point<smithay::utils::geometry::Logical> { x: -762.0, y: 89.5 }
// Determined output size viewport_world: [Rectangle<smithay::utils::geometry::Logical> { x: -2457.028500000001, y: -797.6115000000004, width: 3390.0570000000016, height: 1774.2230000000009 }]
// Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: 133.0, y: -564.0, width: 1226.0, height: 1271.0 }
// Added window with rect_space: Rectangle<smithay::utils::geometry::Logical> { x: -1375.0, y: -546.0, width: 1226.0, height: 1271.0 }
// Calling find with the determined output size and the windows
// Results found: target Position is: (-762.0, 89.5)
