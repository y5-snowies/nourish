//! Embedded picker session: set up / tear down the picker WITHOUT switching the
//! active world, so the sphere can be rendered inside another overlay (the
//! overview's World tab). The picker render/tick/input paths gate on `active`
//! (not on the picker world being active), so a populated `active` is enough to
//! show and drive the globe in place.

use std::time::Instant;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_state_base::base::PickerActive;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

/// Populate a picker session for the current spawn-target world, without
/// switching the active world. No-op if the real picker is active or a session
/// already exists.
pub fn embed_open(state: &mut Loop) {
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return;
    }
    if state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.is_some() {
        return;
    }
    let origin = state.inner.worlds.spawn_target();
    let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    let cell = picker.ensure_cell(origin);
    let faced = compositor_y5_picker_three_orient::orient::face(cell);
    picker.active = Some(PickerActive {
        origin,
        selected: Some(cell),
        pointer: (0.0, 0.0),
        drag: None,
        orientation: faced,
        target: faced,
        spin: compositor_y5_picker_three_orient::orient::IDENTITY,
        zoom: 1.0,
        bevy: None,
        surface: None,
        time: Instant::now(),
        step: Instant::now(),
    });
}

/// Tear down an embedded session (bevy sphere + clear `active`). No-op if the
/// real picker is active or no embed session exists.
pub fn embed_close(state: &mut Loop) {
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return;
    }
    if state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.is_none() {
        return;
    }
    compositor_y5_picker_scene_destroy::destroy::destroy(state);
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active = None;
}

/// Move the focused cell to a grid neighbour (arrow-key navigation), reusing the
/// picker's own neighbour + selection commands. No-op without an embed session.
pub fn select_direction(state: &mut Loop, du: i32, dv: i32) {
    let current = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_ref()
        .and_then(|a| a.selected);
    let Some(current) = current else { return };
    let next = compositor_y5_picker_three_orient::orient::neighbor(current, du, dv);
    compositor_y5_picker_command_base::base::set_selected(state, Some(next));
}

/// Resolve the focused cell's world (creating one for an empty cell, like
/// `picker.start`), for the embedding overlay to switch into. `None` outside an
/// embed session.
pub fn selected_world(state: &mut Loop) -> Option<uuid::Uuid> {
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return None;
    }
    let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    let cell = picker.active.as_ref().and_then(|a| a.selected)?;
    if let Some(world) = picker.cell_worlds.get(cell).copied().flatten() {
        return Some(world);
    }
    let world = compositor_y5_picker_world_base::base::create_world(state);
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).cell_worlds[cell] =
        Some(world);
    compositor_support_system_persist_mark_base::base::mark_world(PICKER_WORLD, true);
    Some(world)
}
