//! Overview World-tab render: the picker globe, embedded.
//!
//! Reuses the picker world's own systems — `embed_open` sets up a session without
//! switching the active world and `scene.create` builds into the picker world's
//! registries. The picker's own tick is skipped (it also owns the parallax, which
//! the overview supplies here), so its steps are re-stated below.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};
use std::cell::RefCell;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_support_bevy_core_compositor_base::{BevyRenderElement, Transform};
use compositor_monitor_compositor_iced_base::{IcedRenderElement, Transform as IcedTransform};
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

/// What the World tab draws: the picker world's bevy sphere and its iced details
/// panel. Two vectors, not one list — different bands of the plan, as in the
/// picker's own `scene.frame`, so the panel sits above the globe.
#[derive(Default)]
pub struct Prepared {
    pub bevy: Vec<BevyRenderElement>,
    pub iced: Vec<IcedRenderElement>,
}

/// Ensure the embedded picker session + sphere exist, advance the orientation, and
/// render both of the picker world's registries.
pub fn prepare_world(
    state: &mut Loop,
    gles: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Prepared {
    // Output size changed (mode/output switch while the overview is open) → tear the
    // embedded globe down so it rebuilds at the new size below.
    let size_changed = GLOBE_SIZE.with(|s| { let mut s = s.borrow_mut(); if *s != Some(size) { *s = Some(size); true } else { false } });
    if size_changed {
        compositor_y5_picker_interface_embed::embed::embed_close(state);
    }
    // Refresh the globe thumbnail as the session opens. The full-screen picker
    // captures the origin world BEFORE opening; here the overlay is already
    // composing, so a capture would photograph the overview. The freeze backdrop is
    // exactly the frame wanted, and this only runs once `overlay_ready()`.
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

    // Panel messages: the picker's tick normally drains them and is skipped here.
    compositor_y5_picker_surface_handle::handle::drain(state);
    // Orientation advance + transform push, shared with the picker's own tick so
    // the globe moves identically either way.
    compositor_y5_picker_command_advance::advance::advance(state);
    state.schedule_redraw_post_vblank();

    let gpu = state.inner.environment.GPU.clone();
    let transform = Transform { zoom: 1.0, position: Point::new(0.0, 0.0) };
    let bevy = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
        .and_then(|b| b.registry.as_mut())
        .and_then(|reg| reg.render_all(&gpu, gles, transform, size.to_f64(), Layer::PICKER_SCENE.bits()).ok())
        .unwrap_or_default();
    // ...and the details panel, built alongside the sphere in the picker world's
    // ICED registry. Rendering only the bevy half is why the tab showed no panel.
    let t = IcedTransform { zoom: 1.0, position: Point::new(0.0, 0.0) };
    let iced = compositor_y5_picker_system_base::base::registry(&mut state.inner.worlds)
        .and_then(|reg| reg.render_all(&gpu, gles, t, size.to_f64(), Layer::PICKER_SCENE.bits()).ok())
        .unwrap_or_default();
    Prepared { bevy, iced }
}
