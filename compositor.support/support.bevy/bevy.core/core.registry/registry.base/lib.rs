//! `BevyRegistry`: the compositor-facing API for all Bevy scene instances
//! (method bodies live in bevy.lifecycle / bevy.order / bevy.frame / bevy.mutate).

use std::collections::HashMap;
use std::sync::Arc;

use compositor_model_debug_instance_record::warn;
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_element_base::BevyRenderElement;
use compositor_support_bevy_core_error_base::{CreateError, DispatchError, ResizeError};
use compositor_support_bevy_core_handle_base::{BevyHandle, HandleId};
use compositor_support_bevy_core_instance_base::BevyInstance;
use compositor_support_bevy_core_item_base::BevyItem;
use compositor_support_bevy_core_scene_base::BevyScene;
use compositor_support_bevy_core_shared_base::SharedContext;
use compositor_support_bevy_core_space_base::{BevySpace, Transform};
use compositor_support_bevy_core_worker_base::{Job, Worker};
use compositor_model_debug_instance_record::info;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};

pub struct BevyRegistry {
    shared: SharedContext,
    wgpu_ctx: Arc<WgpuVulkanContext>,
    items: Vec<BevyItem>,
    index: HashMap<HandleId, usize>,
    next_id: u64,
    last_transform: Transform,
    last_output_size: Size<f64, Physical>,
    instance_scale: f32,
    /// The off-thread host, when this registry is running instances there.
    /// `None` = today's inline path, which is also what GLES always gets.
    worker: Option<Worker>,
}

impl std::fmt::Debug for BevyRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevyRegistry").field("instance_count", &self.items.len()).finish()
    }
}

impl BevyRegistry {
    pub fn new(shared: SharedContext, wgpu_ctx: Arc<WgpuVulkanContext>) -> Self {
        // The worker is Vulkan-only, and deliberately so: its ring slots carry no
        // GLES view, because building one needs `&mut GlesRenderer` — the single
        // thing that cannot leave the compositor thread. On GLES the registry
        // stays inline and behaves exactly as it always has.
        let vulkan = compositor_model_stats_registry_base::base::compositor_prefers_dmabuf();
        let wanted = compositor_model_environment_interface_base::base::get().enabled;
        let worker = compositor_model_environment_interface_base::base::get()
            .engaged()
            .then(|| compositor_support_bevy_core_worker_spawn::spawn(&shared, &wgpu_ctx))
            .flatten();
        if wanted && !vulkan {
            info!("bevy: triple buffering requested but the compositor is on GLES; staying inline");
        }
        Self {
            shared, wgpu_ctx, items: Vec::new(), index: HashMap::new(), next_id: 1,
            last_transform: Transform::identity(), last_output_size: Size::from((0.0, 0.0)), instance_scale: 1.0,
            worker,
        }
    }

    /// Whether instances run on the worker thread.
    pub fn is_offthread(&self) -> bool { self.worker.is_some() }
    pub fn wgpu_ctx(&self) -> &Arc<WgpuVulkanContext> { &self.wgpu_ctx }
    pub fn shared(&self) -> &SharedContext { &self.shared }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
    pub fn contains(&self, id: HandleId) -> bool { self.index.contains_key(&id) }
    pub fn set_instance_scale(&mut self, scale: f32) {
        self.instance_scale = scale;
        compositor_support_bevy_core_mutate_base::set_instance_scale(&mut self.items, scale);
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &BevyItem> + ExactSizeIterator + '_ { self.items.iter() }
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut BevyItem> + ExactSizeIterator + '_ { self.items.iter_mut() }
    pub fn get(&self, id: HandleId) -> Option<&BevyItem> { self.index.get(&id).and_then(|&idx| self.items.get(idx)) }
    pub fn get_mut(&mut self, id: HandleId) -> Option<&mut BevyItem> { let idx = *self.index.get(&id)?; self.items.get_mut(idx) }
    pub fn create<S: BevyScene>(&mut self, render_node: &str, scene: S, gles: &mut GlesRenderer, location: Point<i32, Physical>, size: Size<i32, Physical>, layer: u64) -> Result<BevyHandle<S>, CreateError> {
        self.create_in_space(render_node, scene, gles, location, size, BevySpace::World, layer)
    }
    pub fn create_screen<S: BevyScene>(&mut self, render_node: &str, scene: S, gles: &mut GlesRenderer, location: Point<i32, Physical>, size: Size<i32, Physical>, layer: u64) -> Result<BevyHandle<S>, CreateError> {
        self.create_in_space(render_node, scene, gles, location, size, BevySpace::Screen, layer)
    }
    pub fn create_in_space<S: BevyScene>(&mut self, render_node: &str, scene: S, gles: &mut GlesRenderer, location: Point<i32, Physical>, size: Size<i32, Physical>, space: BevySpace, layer: u64) -> Result<BevyHandle<S>, CreateError> {
        compositor_support_bevy_core_lifecycle_base::create_in_space(&mut self.next_id, &mut self.items, &mut self.index, &self.shared, &self.wgpu_ctx, self.instance_scale, render_node, scene, gles, location, size, space, layer, self.worker.as_ref())
    }
    pub fn destroy<S: BevyScene>(&mut self, handle: BevyHandle<S>) -> bool { self.destroy_by_id(handle.id) }
    pub fn destroy_by_id(&mut self, id: HandleId) -> bool {
        let destroyed =
            compositor_support_bevy_core_lifecycle_base::destroy_by_id(&mut self.items, &mut self.index, id);
        // INLINE PATH ONLY (`worker: None`, i.e. triple buffering off, and always
        // on GLES). Dropping the `App` above only queues its GPU resources; wgpu
        // performs the free during a device poll, and bevy's render loop — the
        // only thing that normally polls — went away with the App.
        //
        // Off-thread, the worker's `Job::Destroy` already does this, which is why
        // the picker's open/close leak looked fixed with triple buffering ON and
        // came straight back when it was turned off. Same bug, second path.
        if destroyed && self.worker.is_none() {
            if let Err(e) = self.wgpu_ctx.device.poll(wgpu::PollType::wait_indefinitely()) {
                warn!("bevy registry: device poll after destroy failed: {e:?}");
            }
        }
        destroyed
    }
    pub fn set_location<S: BevyScene>(&mut self, handle: BevyHandle<S>, location: Point<i32, Physical>) -> bool { self.set_location_by_id(handle.id, location) }
    pub fn set_location_by_id(&mut self, id: HandleId, location: Point<i32, Physical>) -> bool {
        compositor_support_bevy_core_mutate_base::set_location_by_id(&mut self.items, &self.index, id, location)
    }
    pub fn raise(&mut self, id: HandleId) { compositor_support_bevy_core_order_base::raise(&mut self.items, &mut self.index, id) }
    pub fn lower(&mut self, id: HandleId) { compositor_support_bevy_core_order_base::lower(&mut self.items, &mut self.index, id) }
    pub fn location_of(&self, id: HandleId) -> Option<Point<i32, Physical>> { self.get(id).map(|i| i.location()) }
    pub fn size_of(&self, id: HandleId) -> Option<Size<i32, Physical>> { self.get(id).map(|i| i.size()) }
    pub fn space_of(&self, id: HandleId) -> Option<BevySpace> { self.get(id).map(|i| i.space()) }
    pub fn request_resize<S: BevyScene>(&mut self, handle: BevyHandle<S>, new_size: Size<i32, Physical>) -> bool { self.request_resize_by_id(handle.id, new_size) }
    pub fn request_resize_by_id(&mut self, id: HandleId, new_size: Size<i32, Physical>) -> bool {
        compositor_support_bevy_core_mutate_base::request_resize_by_id(&mut self.items, &self.index, id, new_size, self.instance_scale)
    }
    pub fn apply_pending_resizes(&mut self, render_node: &str, gles: &mut GlesRenderer) -> Result<usize, ResizeError> {
        compositor_support_bevy_core_frame_base::apply_pending_resizes(&mut self.items, &self.wgpu_ctx, render_node, gles)
    }
    pub fn dispatch_command<S: BevyScene>(&mut self, handle: BevyHandle<S>, command: S::Command) -> Result<(), DispatchError> {
        // Off-thread there is no local runtime to queue onto: the command is boxed
        // and routed to the worker, which downcasts it back to `S::Command`.
        if let Some(worker) = self.worker.as_ref() {
            if !self.index.contains_key(&handle.id) {
                return Err(DispatchError::UnknownHandle(handle.id));
            }
            worker.send(Job::Command { id: handle.id, payload: Box::new(command) });
            return Ok(());
        }
        compositor_support_bevy_core_mutate_base::dispatch_command(&mut self.items, &self.index, handle, command)
    }
    pub fn instance<S: BevyScene>(&self, handle: BevyHandle<S>) -> Option<&BevyInstance<S>> { self.get(handle.id).and_then(|i| i.get::<S>()) }
    pub fn instance_mut<S: BevyScene>(&mut self, handle: BevyHandle<S>) -> Option<&mut BevyInstance<S>> { self.get_mut(handle.id).and_then(|i| i.get_mut::<S>()) }
    pub fn hit_test(&self, point: Point<f64, Physical>, transform: &Transform, output_size: Size<f64, Physical>) -> Option<HandleId> {
        compositor_support_bevy_core_frame_base::hit_test(&self.items, point, transform, output_size)
    }
    /// One compositor frame. Off-thread this only advances the worker (a single
    /// coalesced tick for the whole registry) and picks up whatever it finished;
    /// inline it renders every instance as before.
    pub fn process_frame(&mut self) {
        if let Some(worker) = self.worker.as_ref() {
            worker.send(Job::Tick);
        }
        compositor_support_bevy_core_mutate_base::process_frame(&mut self.items);
    }
    pub fn elements(&self, transform: &Transform, output_size: Size<f64, Physical>, layer: u64) -> Vec<BevyRenderElement> {
        compositor_support_bevy_core_frame_base::elements(&self.items, transform, output_size, layer)
    }
    pub fn render_all(&mut self, render_node: &str, gles: &mut GlesRenderer, transform: Transform, output_size: Size<f64, Physical>, layer: u64) -> Result<Vec<BevyRenderElement>, ResizeError> {
        self.apply_pending_resizes(render_node, gles)?;
        compositor_support_bevy_core_frame_base::cache_camera_and_bump(&mut self.items, &mut self.last_transform, &mut self.last_output_size, transform, output_size);
        self.process_frame();
        Ok(self.elements(&transform, output_size, layer))
    }
}