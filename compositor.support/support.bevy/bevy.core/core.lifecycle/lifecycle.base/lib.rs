//! Instance lifecycle bodies for `BevyRegistry` (create / destroy).

use std::collections::HashMap;
use std::sync::Arc;

use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_error_base::CreateError;
use compositor_support_bevy_core_handle_base::{BevyHandle, HandleId};
use compositor_support_bevy_core_host_base::BevyRuntime;
use compositor_support_bevy_core_instance_base::BevyInstance;
use compositor_support_bevy_core_item_base::BevyItem;
use compositor_support_bevy_core_scene_base::BevyScene;
use compositor_support_bevy_core_shared_base::SharedContext;
use compositor_support_bevy_core_space_base::BevySpace;
use compositor_support_bevy_core_surface_base::BevySurface;
use compositor_model_debug_instance_record::trace;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};

#[allow(clippy::too_many_arguments)]
pub fn create_in_space<S: BevyScene>(
    next_id: &mut u64,
    items: &mut Vec<BevyItem>,
    index: &mut HashMap<HandleId, usize>,
    shared: &SharedContext,
    wgpu_ctx: &Arc<WgpuVulkanContext>,
    instance_scale: f32,
    render_node: &str,
    scene: S,
    gles: &mut GlesRenderer,
    location: Point<i32, Physical>,
    size: Size<i32, Physical>,
    space: BevySpace,
    layer: u64,
) -> Result<BevyHandle<S>, CreateError> {
    let id = HandleId(*next_id);
    *next_id += 1;

    // Output surface (Bevy renders into it, compositor samples it).
    let output_surface = BevySurface::allocate(render_node, wgpu_ctx, gles, size)?;
    let output_wgpu_tex = Arc::new(output_surface.wgpu_texture.clone());

    // Build the runtime — no inputs managed here. The scene's own
    // constructor carries any dmabuf-imported wgpu textures it needs.
    let runtime = BevyRuntime::new(
        scene,
        shared.clone(),
        output_wgpu_tex,
        (size.w as u32, size.h as u32),
        instance_scale,
    );

    let instance = BevyInstance {
        id,
        smithay_id: smithay::backend::renderer::element::Id::new(),
        commit: smithay::backend::renderer::utils::CommitCounter::default(),
        location,
        output_surface,
        scale_factor: instance_scale,
        runtime,
        pending_resize: None,
    };

    trace!("created bevy instance handle={id:?} location={location:?} size={size:?} space={space:?}");

    let item = BevyItem::new(instance, space, layer);
    let idx = items.len();
    items.push(item);
    index.insert(id, idx);
    Ok(BevyHandle::new(id))
}

pub fn destroy_by_id(
    items: &mut Vec<BevyItem>,
    index: &mut HashMap<HandleId, usize>,
    id: HandleId,
) -> bool {
    let Some(idx) = index.remove(&id) else {
        return false;
    };
    items.remove(idx);
    for (_, i) in index.iter_mut() {
        if *i > idx {
            *i -= 1;
        }
    }
    trace!("destroyed bevy instance handle={id:?}");
    true
}
