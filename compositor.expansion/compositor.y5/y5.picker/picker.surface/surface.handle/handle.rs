//! Compositor-side handling of picker panel messages (drained from the surface
//! channel each picker frame).

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_surface_view::PickerSurfaceMessage;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

pub fn delegate(state: &mut Loop, message: PickerSurfaceMessage) {
    match message {
        PickerSurfaceMessage::NameEdited(name) => rename_selected(state, name),
        PickerSurfaceMessage::Enter => compositor_y5_picker_world_base::base::start(state),
        PickerSurfaceMessage::DeleteConfirm => {
            compositor_y5_picker_world_delete::delete::delete(state)
        }
        PickerSurfaceMessage::Close => compositor_y5_picker_interface_base::base::cancel(state),
        PickerSurfaceMessage::SetWorld { .. }
        | PickerSurfaceMessage::DeleteRequest
        | PickerSurfaceMessage::DeleteCancel => {}
    }
}

/// Store the edited name against the selected cell's world (no-op on empty cell).
fn rename_selected(state: &mut Loop, name: String) {
    let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    let Some(cell) = picker.active.as_ref().and_then(|a| a.selected) else {
        return;
    };
    if let Some(world) = picker.cell_worlds.get(cell).copied().flatten() {
        picker.world_names.insert(world, name);
    }
}
