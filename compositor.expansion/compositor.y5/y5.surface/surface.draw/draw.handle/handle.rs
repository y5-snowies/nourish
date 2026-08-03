use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use std::sync::{Arc, mpsc};
use compositor_orchestration_core_state_base::Loop;
use compositor_monitor_compositor_iced_base::{IcedHandle, IcedRegistry};

use compositor_support_iced_core_engine_base::{EngineSettings, IcedSnapshot, IcedUi, SharedEngine};

pub use compositor_monitor_compositor_iced_base::IcedSpace;

/// [`load`] for a UI the compositor reads back synchronously via
/// `IcedRegistry::snapshot`. Identical placement and registration; the only
/// difference is that the off-thread worker publishes a copy of the UI's state
/// each tick, which the compositor reads instead of borrowing the runtime.
pub fn load_snapshot<T: IcedSnapshot>(
    state: &mut Loop,
    gles: &mut GlesRenderer,
    t: T,
    rect: Rectangle<i32, Physical>,
    space: IcedSpace,
    layer: u64,
) -> IcedHandle<T> {
    let gpu = state.inner.environment.GPU.clone();
    let handle = {
        let registry = state.inner.surface_mut()
            .registry
            .as_mut()
            .unwrap_or_else(|| abort!("registry to be created"));
        registry
            .create_in_space_snapshot(&gpu.as_str(), t, gles, rect.loc, rect.size, space, layer)
            .unwrap()
    };
    register(state, handle, space, layer);
    handle
}

/// The draw-order registration both `load` variants share.
///
/// WORLD-space iced surfaces are registered in the renderer-agnostic draw order
/// so they interleave with windows by DrawOrder ("everything" interleaves);
/// screen-space iced is a screen-locked overlay and keeps its own band, so it is
/// not registered at all. The drawable id is derived from the iced `HandleId`
/// and is reversible: `HandleId(uuid.as_u128() as u64)`.
///
/// The layer mask picks the tier: a group frame sits beneath the windows it
/// contains, everything else is CONTENT.
fn register<T: IcedUi>(state: &mut Loop, handle: IcedHandle<T>, space: IcedSpace, layer: u64) {
    if let IcedSpace::World = space {
        let tier = if layer & compositor_orchestration_draw_layer_base::base::Layer::SCENE_SURFACE_GROUP.bits() != 0 {
            compositor_support_world_order_track_base::base::DrawLayer::GROUP
        } else {
            compositor_support_world_order_track_base::base::DrawLayer::CONTENT
        };
        state.inner.register_drawable(uuid::Uuid::from_u128(handle.id.0 as u128), tier);
    }
}

pub fn load<T: IcedUi>(
    state: &mut Loop,
    gles: &mut GlesRenderer,
    t: T,
    rect: Rectangle<i32, Physical>,
    space: IcedSpace,
    layer: u64,
) -> IcedHandle<T> {
    // Hoist GPU before the surface-registry borrow (surface_mut borrows all of inner).
    let gpu = state.inner.environment.GPU.clone();
    let registry = state.inner.surface_mut()
        .registry
        .as_mut()
        .unwrap_or_else(|| abort!("registry to be created"));

    let handle = match space {
        IcedSpace::World => registry
            .create(
                &gpu.as_str(),
                t,
                gles,
                rect.loc,
                rect.size,
                layer,
            )
            .unwrap(),
        IcedSpace::Screen => registry
            .create_screen(
                &gpu.as_str(),
                t,
                gles,
                rect.loc,
                rect.size,
                layer,
            )
            .unwrap(),
    };

    register(state, handle, space, layer);
    handle
}
