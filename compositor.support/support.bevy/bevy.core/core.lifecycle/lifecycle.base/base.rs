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
use compositor_support_bevy_core_worker_base::{AnyRuntime, Factory, Job, Worker};
use compositor_support_bevy_core_worker_instance::WorkerInstance;
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
    worker: Option<&Worker>,
) -> Result<BevyHandle<S>, CreateError> {
    let id = HandleId(*next_id);
    *next_id += 1;

    // Off-thread path. Construction is OPTIMISTIC: the handle is returned now and
    // the worker builds the instance on its own thread, so an allocation failure
    // surfaces as a log a frame or two later rather than as an `Err` here. That is
    // the trade for getting the `App` — and its GPU work — off the input thread.
    if let Some(worker) = worker {
        // The scene moves into the closure; the `App` it becomes is built on the
        // worker thread from the context the worker hands in. Nothing `!Send`
        // crosses — that is the entire reason this is a factory and not a value.
        let factory: Factory = Box::new(move |ctx, texture, size_px, scale| {
            Box::new(BevyRuntime::new(scene, ctx.clone(), texture, size_px, scale)) as Box<dyn AnyRuntime>
        });
        worker.send(Job::Create { id, size, scale: instance_scale, factory });
        let instance = WorkerInstance {
            id,
            smithay_id: smithay::backend::renderer::element::Id::new(),
            commit: smithay::backend::renderer::utils::CommitCounter::default(),
            location,
            size,
            scale_factor: instance_scale,
            pending_resize: None,
            generation: 0,
            worker: worker.clone(),
            published: None,
        };
        trace!("created remote bevy instance handle={id:?} size={size:?} space={space:?}");
        let item = BevyItem::remote(instance, space, layer);
        let idx = items.len();
        items.push(item);
        index.insert(id, idx);
        return Ok(BevyHandle::new(id));
    }

    // Output surface (Bevy renders into it, compositor samples it).
    // Bevy is pointed at the slot the ring starts on; `tick` re-points it as the
    // ring rotates (and not at all when the ring is a single slot).
    let output_surface = BevySurface::allocate(render_node, wgpu_ctx, gles, size)?;
    let output_wgpu_tex = Arc::new(output_surface.target().wgpu_texture.clone());

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
        generation: 0,
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
