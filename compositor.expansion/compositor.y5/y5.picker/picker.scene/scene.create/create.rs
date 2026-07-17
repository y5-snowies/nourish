use std::sync::Arc;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use compositor_y5_picker_three_scene::{PickerCommand, PickerScene};

/// Build the sphere scene (PICKER world's own registry) + the details panel
/// (session registry). No-op if the picker isn't active / bevy isn't ready.
pub fn create(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Bake at the ACTIVE output's size (the monitor the picker opens on and only
    // renders on), not whichever output's pass triggered this open — a wrong size
    // forced a first-frame resize that raced the bevy first render and blanked it.
    let size = state.inner.active_output().current_mode().map(|m| m.size).unwrap_or(size);

    // Per-cell thumbnails (None → transparent), occupancy (cell holds a world,
    // even with no thumbnail — e.g. restored from disk), + cell to focus.
    let (thumbnails, occupied, selected): (Vec<Option<Arc<wgpu::Texture>>>, Vec<bool>, Option<usize>) = {
        let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
        let Some(active) = picker.active.as_ref() else {
            return;
        };
        let selected = active.selected;
        let occupied = picker.cell_worlds.iter().map(|c| c.is_some()).collect();
        let thumbnails = picker.cell_worlds.iter()
            .map(|cell| cell.and_then(|w| picker.thumbnails.get(&w).map(|s| s.wgpu_texture())))
            .collect();
        (thumbnails, occupied, selected)
    };
    let scene = PickerScene::new((size.w.max(1) as u32, size.h.max(1) as u32), thumbnails, occupied);

    // The picker world OWNS its bevy registry, pre-created at startup by the
    // loader prewarm pass — asserted present here rather than built mid-render.
    let gpu = state.inner.environment.GPU.clone();
    let handle = {
        let Some(registry) = state
            .inner
            .worlds
            .get_mut(PICKER_WORLD)
            .storage_mut()
            .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
            .and_then(|b| b.registry.as_mut())
        else {
            abort!("picker: bevy registry missing — startup prewarm failed");
        };
        match registry.create_screen(
            &gpu,
            scene,
            renderer,
            Point::from((0, 0)),
            size,
            Layer::PICKER_SCENE.bits(),
        ) {
            Ok(handle) => {
                // Focus the origin cell immediately (auto-select the world we
                // came from): outline + "+" appear on it.
                let _ = registry.dispatch_command(handle, PickerCommand::SetSelected(selected));
                handle
            }
            Err(e) => {
                error!("picker: create_screen failed: {e:?}");
                return;
            }
        }
    };

    if let Some(active) = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .get_mut(&PICKER_MUT)
        .active
        .as_mut()
    {
        active.bevy = Some(handle);
        info!("picker: sphere scene created");
    }

    // The bottom-right details panel (iced, in the session registry).
    if let Some(s) = compositor_y5_picker_surface_create::create::create(state, renderer, size) {
        let picker = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
        if let Some(active) = picker.active.as_mut() {
            active.surface = Some(s);
        }
        compositor_y5_picker_command_base::base::sync_surface(state); // show name + delete-availability
    }
}
