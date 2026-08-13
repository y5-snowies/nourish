use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use compositor_y5_picker_three_scene::PickerCommand;

/// Dispatch a command to the picker's bevy scene instance. No-op if absent.
pub fn dispatch(state: &mut Loop, command: PickerCommand) {
    let handle = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_ref()
        .and_then(|a| a.bevy);
    let Some(handle) = handle else {
        return;
    };
    if let Some(reg) = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
        .and_then(|b| b.registry.as_mut())
    {
        let _ = reg.dispatch_command(handle, command);
    }
}

/// Push the picker's current transform (orientation + zoom) to the scene.
pub fn push_transform(state: &mut Loop) {
    let t = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_ref()
        .map(|a| (a.orientation.quat(), a.zoom));
    if let Some((orientation, zoom)) = t {
        dispatch(state, PickerCommand::SetTransform { orientation, zoom });
    }
}

/// Set the focused cell + re-face the sphere to it, syncing scene + panel.
pub fn set_selected(state: &mut Loop, cell: Option<usize>) {
    if let Some(active) = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_mut()
    {
        active.selected = cell;
        if let Some(c) = cell {
            // Animate (don't snap) toward facing the chosen cell.
            active.target = compositor_y5_picker_three_orient::orient::face(c, active.orientation.yaw);
            active.spin = compositor_y5_picker_three_orient::orient::Orient::ZERO;
        }
    }
    dispatch(state, PickerCommand::SetSelected(cell));
    sync_surface(state);
}

/// Push the focused world's name + delete-availability to the details panel.
pub fn sync_surface(state: &mut Loop) {
    let (handle, name, can_delete) = {
        let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
        let Some((handle, selected)) = picker.active.as_ref().map(|a| (a.surface, a.selected)) else {
            return;
        };
        let Some(handle) = handle else {
            return;
        };
        let world = selected.and_then(|c| picker.cell_worlds.get(c).copied().flatten());
        let name = world.map(|w| picker.world_names.entry(w)
            .or_insert_with(|| compositor_y5_picker_name_pool::pool::random_name(w)).clone()).unwrap_or_default();
        let can_delete =
            world.is_some() && picker.cell_worlds.iter().filter(|c| c.is_some()).count() > 1;
        (handle, name, can_delete)
    };
    // `world_names` may have been lazily assigned above — persist it (debounced).
    compositor_support_system_persist_mark_base::base::mark_world(PICKER_WORLD, false);
    if let Some(reg) = compositor_y5_picker_system_base::base::registry(&mut state.inner.worlds) {
        let _ = reg.dispatch_message(
            handle,
            compositor_y5_picker_surface_view::PickerSurfaceMessage::SetWorld { name, can_delete },
        );
    }
}
