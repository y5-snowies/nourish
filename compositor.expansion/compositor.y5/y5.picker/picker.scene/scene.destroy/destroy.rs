use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

/// Tear down the picker's bevy instance + details panel on exit
/// (`interface.base::enter`).
pub fn destroy(state: &mut Loop) {
    // The picker overlay going away. Its backdrop pane hangs off PICKER_WORLD,
    // which is never a spawn target and so is never named by a world
    // retirement — this is its only deterministic close.
    //
    // Guarded, because the overview's embedded World-tab globe tears down
    // through here too, and the backdrop up in that case is the overview's.
    if state.inner.worlds.active_id() == PICKER_WORLD {
        compositor_kernel_graphic_bridge_publish_retire::retire::retire_overlay("picker");
    }
    let (bevy_id, surface_id) = {
        let a = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.as_ref();
        (a.and_then(|a| a.bevy.map(|h| h.id)), a.and_then(|a| a.surface.map(|h| h.id)))
    };

    // Bevy sphere — picker world's own registry.
    if let Some(id) = bevy_id {
        if let Some(reg) = state
            .inner
            .worlds
            .get_mut(PICKER_WORLD)
            .storage_mut()
            .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
            .and_then(|b| b.registry.as_mut())
        {
            reg.destroy_by_id(id);
        }
    }
    // Details panel — session iced registry.
    if let Some(id) = surface_id {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.destroy_by_id(id);
        }
    }
}
