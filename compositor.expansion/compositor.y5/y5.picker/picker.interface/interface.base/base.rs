use std::time::Instant;

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_state_base::base::PickerActive;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

/// Open the world-selection screen. Remember the world we came from, then switch
/// the active binding to the PICKER overlay world. Like lock, this only moves
/// `active` — `spawn_target` (and therefore every window) stays on the session
/// world, which is merely suspended. The normal entry arrives via
/// `interface.capture::finish_arm_and_open` (after the origin thumbnail is taken).
pub fn open(state: &mut Loop) {
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return;
    }
    // The SESSION world, not `active_id()`: this open is deferred over several
    // frames, so `active` may have become an OVERLAY world (lock) in between — and
    // `ensure_cell` below writes `origin` into the grid, which holds scene worlds
    // only. `spawn_target` is the world the picker was opened from either way.
    let origin = state.inner.worlds.spawn_target();

    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        worlds.switch(PICKER_WORLD, kernel);
    }

    // Map the origin world to a cell (persistently), focus it, and orient the
    // sphere so that cell faces the camera (the initial re-face).
    let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    let cell = picker.ensure_cell(origin);
    let selected = Some(cell);
    let faced = compositor_y5_picker_three_orient::orient::face(cell, 0.0);
    picker.active = Some(PickerActive {
        origin,
        selected,
        pointer: (0.0, 0.0),
        drag: None,
        orientation: faced,
        target: faced,
        spin: compositor_y5_picker_three_orient::orient::Orient::ZERO,
        zoom: 1.0,
        bevy: None,
        surface: None,
        time: Instant::now(),
        step: Instant::now(),
    });

    info!("picker: open (origin world {origin}, cell {selected:?})");
}

/// Cancel the picker and return to the world it was opened from.
pub fn cancel(state: &mut Loop) {
    if state.inner.worlds.active_id() != PICKER_WORLD {
        return;
    }
    let origin = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_ref()
        .map(|a| a.origin)
        .unwrap_or_else(|| state.inner.worlds.spawn_target());

    enter(state, origin);
}

/// Leave the picker for `target` (the chosen world). Clears the picker's active
/// state and switches the active binding. Phase 1/2 only ever target an existing
/// world; lazy creation of a brand-new world for an empty cell lands in Phase 4.
pub fn enter(state: &mut Loop, target: uuid::Uuid) {
    if state.inner.worlds.active_id() != PICKER_WORLD {
        return;
    }

    // Tear down the bevy sphere instance from the session world's registry.
    compositor_y5_picker_scene_destroy::destroy::destroy(state);

    state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active = None;

    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        worlds.switch(target, kernel);
    }
    info!("picker: enter world {target}");
}
