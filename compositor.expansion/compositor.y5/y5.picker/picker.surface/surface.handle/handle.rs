//! Compositor-side handling of picker panel messages (drained from the surface
//! channel each picker frame).

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_surface_view::PickerSurfaceMessage;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

/// Drain the PICKER world's surface channel and act on its panel messages.
///
/// Every path that shows the panel owes this call — the panel publishes to its own
/// world's channel, and nothing else drains it. `scene.tick` does it for the
/// full-screen picker; the overview's World tab runs its own prepare and skips that
/// tick entirely, which is why renaming a world and confirming a delete did nothing
/// there: the messages were queued and never read.
pub fn drain(state: &mut Loop) {
    let messages: Vec<_> = {
        let mut v = Vec::new();
        if let Some(s) = compositor_y5_picker_system_base::base::surface(&mut state.inner.worlds) {
            while let Ok(m) = s.surface_message_buffer_channel.1.try_recv() {
                v.push(m);
            }
        }
        v
    };
    for m in messages {
        if let compositor_y5_surface_protocol_base::protocol::SurfaceMessageType::Picker(pm) =
            m.message
        {
            delegate(state, pm);
        }
    }
}

pub fn delegate(state: &mut Loop, message: PickerSurfaceMessage) {
    match message {
        PickerSurfaceMessage::NameEdited(name) => rename_selected(state, name),
        PickerSurfaceMessage::Enter => compositor_y5_picker_world_base::base::start(state),
        PickerSurfaceMessage::DeleteConfirm => {
            compositor_y5_picker_world_delete::delete::delete(state)
        }
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
