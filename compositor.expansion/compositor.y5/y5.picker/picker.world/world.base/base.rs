use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use smithay::utils::Point;

/// Lazily build a new spatial world (the stock system set minus `ThreeSystem` —
/// the bevy context is single-owner, so per-world 3D is omitted, as for the
/// loader's test worlds), add it to the `WorldManager`, and return its id.
pub fn create_world(state: &mut Loop) -> uuid::Uuid {
    create_world_with_id(state, uuid::Uuid::now_v7())
}

/// Build a spatial world with a SPECIFIC id (used to recreate a persisted world
/// under its saved UUID, so its per-world state + placeholders reload).
pub fn create_world_with_id(state: &mut Loop, id: uuid::Uuid) -> uuid::Uuid {
    use compositor_support_system_trait_system_base::base::System;
    let systems: Vec<Box<dyn System>> = vec![
        Box::new(compositor_y5_navigator_system_base::base::NavigatorSystem),
        Box::new(compositor_y5_camera_system_base::base::CameraSystem::default()),
        Box::new(compositor_background_two_system_base::base::TwoSystem),
        Box::new(compositor_y5_window_system_base::base::WindowSystem),
        Box::new(compositor_y5_surface_system_base::base::SurfaceSystem),
        Box::new(compositor_y5_canvas_system_base::base::CanvasSystem),
        Box::new(compositor_orchestration_seat_system_pointer::base::PointerSystem),
        Box::new(compositor_y5_placeholder_system_base::base::PlaceholderSystem),
        Box::new(compositor_y5_launcher_system_base::base::LauncherSystem),
        Box::new(compositor_y5_select_system_base::base::SelectSystem),
        Box::new(compositor_y5_select_overlay_system::base::SelectionOverlaySystem),
        Box::new(compositor_y5_group_system_base::base::GroupSystem),
        // Spatial worlds can become the spawn-target, where `overview()` reads the
        // OVERVIEW slot every frame — so every spatial world must seed it.
        Box::new(compositor_y5_overview_system_base::base::OverviewSystem),
    ];
    let world = compositor_support_world_kind_build_base::base::spatial(id, "world", systems, &state.inner.kernel);
    let added = state.inner.worlds.add(world);
    // Pre-create the new world's iced registry off the render path (the shared GPU
    // context is guaranteed present post-startup); no ThreeSystem here, so no bevy.
    compositor_y5_surface_system_base::base::ensure_registry(state.inner.worlds.get_mut(added).storage_mut(), &state.inner.kernel);
    info!("picker: created world {added}");
    added
}

/// Enter the focused cell's world as the new session world — creating it if the
/// cell is empty — and make it the spawn-target. No-op outside the picker or
/// with nothing focused.
pub fn start(state: &mut Loop) {
    if state.inner.worlds.active_id() != PICKER_WORLD {
        return;
    }
    let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    let Some(cell) = picker.active.as_ref().and_then(|a| a.selected) else {
        return;
    };
    let existing = picker.cell_worlds.get(cell).copied().flatten();
    let target = match existing {
        Some(world) => world,
        None => {
            info!("picker.start: creating world for empty cell {cell}");
            let world = create_world(state);
            state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).cell_worlds[cell] =
                Some(world);
            // Picker owns the `world` table; creation is rare → persist IMMEDIATELY.
            compositor_support_system_persist_mark_base::base::mark_world(PICKER_WORLD, true);
            world
        }
    };
    info!("picker.start: cell {cell} -> world {target}; switching");

    // Grab every current output WITH its global-space position (on the old session
    // space) before switching — multi-output: the target world's fresh Space must
    // receive all of them at their real (tiled) positions, not just the first at 0.
    let outputs: Vec<(smithay::output::Output, Point<i32, smithay::utils::Logical>)> =
        state.inner.space_state().state.outputs().map(|o| {
            let loc = state.inner.space_state().state.output_geometry(o).map(|g| g.loc).unwrap_or_default();
            (o.clone(), loc)
        }).collect();

    // Tear down the picker scene + clear active + switch, then make `target` the
    // spawn-target so new windows map into it, mapping the outputs on first entry.
    compositor_y5_picker_interface_base::base::enter(state, target);
    state.inner.worlds.set_spawn_target(target);
    info!("picker.start: spawn-target set to {target}");
    if state.inner.space_state().state.outputs().next().is_none() {
        for (output, loc) in &outputs {
            state.inner.space_state_mut().state.map_output(output, *loc);
        }
        info!("picker.start: mapped {} output(s) into world {target}", outputs.len());
    }
    info!("picker.start: entered world {target} OK");
}
