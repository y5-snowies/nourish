//! Delete the selected cell's world: transfer its windows to the next available
//! world and free the cell. Blocked when it is the only cell world (there must
//! always be one). Worlds never close — "delete" empties the cell + moves windows.

use compositor_orchestration_core_state_base::Loop;
use compositor_support_world_host_space_base::base::{SPACE, SPACE_MUT};
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

pub fn delete(state: &mut Loop) {
    let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    let Some(cell) = picker.active.as_ref().and_then(|a| a.selected) else {
        return;
    };
    let Some(world) = picker.cell_worlds.get(cell).copied().flatten() else {
        return;
    };
    // There must always be at least one cell world.
    let occupied = picker.cell_worlds.iter().filter(|c| c.is_some()).count();
    if occupied <= 1 {
        warn!("picker: refusing to delete the only world");
        return;
    }
    // The next available world to receive the windows.
    let Some(target) = picker.cell_worlds.iter().flatten().copied().find(|&w| w != world) else {
        return;
    };
    picker.cell_worlds[cell] = None;
    picker.world_names.remove(&world);
    if let Some(active) = picker.active.as_mut() {
        if active.origin == world {
            active.origin = target;
        }
    }
    // The picker owns the `world` table; world removal is rare + important, so
    // persist it IMMEDIATELY. The moved-in placeholders re-link under `target` when
    // its world next commits.
    compositor_support_system_persist_mark_base::base::mark_world(PICKER_WORLD, true);

    transfer_windows(state, world, target);
    if state.inner.worlds.spawn_target() == world {
        state.inner.set_spawn_target_world(target);
    }
    info!("picker: deleted world {world} (cell {cell}); windows -> world {target}");
}

/// Move all windows from `from`'s space into `to`'s space.
fn transfer_windows(state: &mut Loop, from: uuid::Uuid, to: uuid::Uuid) {
    let moved: Vec<(smithay::desktop::Window, smithay::utils::Point<i32, smithay::utils::Logical>)> = {
        let space = &state.inner.worlds.get(from).storage().get(&SPACE).inner.state;
        space
            .elements()
            .map(|w| (w.clone(), space.element_location(w).unwrap_or_default()))
            .collect()
    };
    for (w, loc) in &moved {
        state.inner.worlds.get_mut(to).storage_mut().get_mut(&SPACE_MUT).inner.state.map_element(
            w.clone(),
            *loc,
            false,
        );
    }
    let from_space = &mut state.inner.worlds.get_mut(from).storage_mut().get_mut(&SPACE_MUT).inner.state;
    for (w, _) in &moved {
        from_space.unmap_elem(w);
    }
}
