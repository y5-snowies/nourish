//! Overview activation: the actions a click/Enter performs from the overlay —
//! Layout's "view this window" and World's "enter this world". Both close the
//! overlay first.

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;

/// Layout tab: close the overlay and travel the camera to fit the clicked
/// window (the "view" action). No-op if the uuid no longer maps to a window.
pub fn activate(state: &mut Loop, uuid: uuid::Uuid) {
    let window = state
        .inner
        .space_state()
        .state
        .elements()
        .find(|w| w.uuid() == Some(uuid))
        .cloned();
    let Some(window) = window else { return };
    compositor_y5_overview_interface_base::base::request_close(state);
    compositor_y5_navigator_interface_base::interface::fit_to_window(state, &window);
}

/// World tab: enter the focused globe cell's world. Resolve (or create) the
/// world via the embedded picker, tear everything down synchronously (the globe
/// and the menu-bar surface — destroy needs no renderer), then switch the
/// session to it (mirrors `picker.start`). No-op without a focused world.
pub fn activate_world(state: &mut Loop) {
    let Some(target) = compositor_y5_picker_interface_embed::embed::selected_world(state) else {
        return;
    };
    compositor_y5_picker_interface_embed::embed::embed_close(state);
    compositor_y5_overview_interface_surface::surface::close(state);
    state.inner.overview_mut().visible = false;

    // Capture EVERY mapped output with its global-space position before the world
    // switch — multi-output: the target world's fresh Space must receive all of
    // them at their real (tiled) positions, not just the first at the origin.
    let outputs: Vec<(smithay::output::Output, smithay::utils::Point<i32, smithay::utils::Logical>)> =
        state.inner.space_state().state.outputs().map(|o| {
            let loc = state.inner.space_state().state.output_geometry(o).map(|g| g.loc).unwrap_or_default();
            (o.clone(), loc)
        }).collect();
    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        worlds.switch(target, kernel);
    }
    state.inner.worlds.set_spawn_target(target);
    if state.inner.space_state().state.outputs().next().is_none() {
        for (output, loc) in &outputs {
            state.inner.space_state_mut().state.map_output(output, *loc);
        }
    }
}
