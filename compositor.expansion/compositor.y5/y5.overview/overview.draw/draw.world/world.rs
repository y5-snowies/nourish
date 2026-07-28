//! Overview World-tab render: the picker globe, embedded.
//!
//! Reuses the picker world's own systems — `embed_open` sets up a picker session
//! without switching the active world, `scene.create` builds the sphere in the
//! picker world's bevy registry, and `render_all` draws it. The orientation step
//! comes from `picker.command/command.advance` (the picker's own tick is skipped
//! because it drains the shared surface channel and the parallax, which the
//! overview owns).

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};
use std::cell::RefCell;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_support_bevy_core_compositor_base::{BevyRenderElement, Transform};
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

thread_local! {
    /// Output size the embedded globe scene was last built for. The sphere scene
    /// (camera/projection) is created once at this size; on an output/mode change
    /// it must be rebuilt, or the globe renders at a stale aspect.
    static GLOBE_SIZE: RefCell<Option<Size<i32, Physical>>> = const { RefCell::new(None) };
}

/// The picker world's state slot (the embed session lives there, not here).
fn picker(state: &mut Loop) -> &mut compositor_y5_picker_state_base::base::PickerState {
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT)
}

/// Ensure the embedded picker session + sphere exist, advance the orientation,
/// and render the picker world's bevy registry. Returns the bevy elements.
pub fn prepare_world(
    state: &mut Loop,
    gles: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Vec<BevyRenderElement> {
    // Output size changed (mode/output switch while the overview is open) → tear the
    // embedded globe down so it rebuilds at the new size below.
    let size_changed = GLOBE_SIZE.with(|s| { let mut s = s.borrow_mut(); if *s != Some(size) { *s = Some(size); true } else { false } });
    if size_changed {
        compositor_y5_picker_interface_embed::embed::embed_close(state);
    }
    // Refresh the current world's globe thumbnail as the session opens. The
    // full-screen picker arms a framebuffer capture of the origin world BEFORE it
    // opens; the embed path has no such moment — by the time the World tab
    // renders, the overlay is already composing, so a capture taken here would
    // photograph the overview. Without this the cell kept whatever the full-screen
    // picker last stored (often frames old, or nothing at all). The overview's own
    // freeze backdrop is exactly the frame wanted — the desktop as it looked
    // before the overlay drew — and `prepare` only reaches here once
    // `overlay_ready()`, so it is always resolved by now.
    let opening = picker(state).active.is_none();
    let frozen = if opening { state.inner.overview().frozen().cloned() } else { None };
    if let Some(snapshot) = frozen {
        let world = state.inner.worlds.spawn_target();
        picker(state).thumbnails.insert(world, snapshot);
    }
    compositor_y5_picker_interface_embed::embed::embed_open(state);

    // Build the sphere scene once (the picker world's own registry).
    if !picker(state).active.as_ref().is_some_and(|a| a.bevy.is_some()) {
        compositor_y5_picker_scene_create::create::create(state, gles, size);
    }

    // Orientation advance + transform push. Shared with the picker's own tick
    // (which additionally drains the surface channel and the parallax, both
    // owned by the overview here) so the globe moves identically either way.
    compositor_y5_picker_command_advance::advance::advance(state);
    state.schedule_redraw_post_vblank();

    let gpu = state.inner.environment.GPU.clone();
    let reg = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
        .and_then(|b| b.registry.as_mut());
    let Some(reg) = reg else { return Vec::new() };
    let transform = Transform { zoom: 1.0, position: Point::new(0.0, 0.0) };
    reg.render_all(&gpu, gles, transform, size.to_f64(), Layer::PICKER_SCENE.bits())
        .unwrap_or_default()
}
