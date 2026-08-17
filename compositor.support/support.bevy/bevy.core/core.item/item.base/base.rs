use std::any::TypeId;
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_element_base::BevyRenderElement;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_instance_base::{BevyInstance, BevyInstanceAny};
use compositor_support_bevy_core_scene_base::BevyScene;
use compositor_support_bevy_core_space_base::{BevySpace, Transform, item_screen_location, item_screen_size};
use compositor_kernel_graphic_bridge_publish_attempt::attempt::Attempt;
use compositor_support_bevy_core_worker_instance::WorkerInstance;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Point, Rectangle, Size};

pub struct BevyItem {
    pub inner: Box<dyn BevyInstanceAny>,
    pub layer: u64,
    type_id: TypeId,
    space: BevySpace,
    /// The ring-depth request that failed, if any. The wanted depth does not
    /// change when the allocation fails, so an ungated retry runs once per
    /// instance per composite. Keyed on `(depth, size)`: those are the only two
    /// inputs whose change could make the driver answer differently.
    depth_failed: Attempt<(usize, Size<i32, Physical>)>,
}

impl BevyItem {
    #[doc(hidden)]
    pub fn new<S: BevyScene>(inst: BevyInstance<S>, space: BevySpace, layer: u64) -> Self {
        Self { type_id: TypeId::of::<BevyInstance<S>>(), inner: Box::new(inst), space, layer, depth_failed: Attempt::new() }
    }

    /// An instance whose `App` lives on the worker thread. Its `type_id` is the
    /// stand-in's, not `BevyInstance<S>`'s, so `get::<S>()` correctly returns
    /// `None`: there is no local runtime to hand out, and commands must go
    /// through the registry's channel rather than a direct borrow.
    #[doc(hidden)]
    pub fn remote(inst: WorkerInstance, space: BevySpace, layer: u64) -> Self {
        Self { type_id: TypeId::of::<WorkerInstance>(), inner: Box::new(inst), space, layer, depth_failed: Attempt::new() }
    }

    pub fn handle_id(&self) -> HandleId { self.inner.handle_id() }
    pub fn space(&self) -> BevySpace { self.space }
    pub fn set_space(&mut self, space: BevySpace) {
        if self.space != space { self.space = space; self.inner.bump_commit(); }
    }

    pub fn location(&self) -> Point<i32, Physical> { self.inner.location() }
    pub fn size(&self) -> Size<i32, Physical> { self.inner.size() }
    pub fn set_location(&mut self, p: Point<i32, Physical>) { self.inner.set_location(p); }
    pub fn screen_location(&self, transform: &Transform, output_size: Size<f64, Physical>) -> Point<i32, Physical> {
        item_screen_location(self.space, transform, output_size, self.inner.location())
    }

    pub fn screen_size(&self, transform: &Transform) -> Size<i32, Physical> {
        item_screen_size(self.space, transform, self.inner.size())
    }

    pub fn screen_rect(&self, transform: &Transform, output_size: Size<f64, Physical>) -> Rectangle<i32, Physical> {
        Rectangle::from_loc_and_size(self.screen_location(transform, output_size), self.screen_size(transform))
    }

    pub fn contains_screen_point(&self, point: Point<f64, Physical>, transform: &Transform, output_size: Size<f64, Physical>) -> bool {
        let r = self.screen_rect(transform, output_size);
        let r = Rectangle::<f64, Physical>::from_loc_and_size((r.loc.x as f64, r.loc.y as f64), (r.size.w as f64, r.size.h as f64));
        r.contains(point)
    }

    pub fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32) { self.inner.request_resize(new_size, scale_factor); }
    pub fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)> { self.inner.pending_resize() }
    pub fn commit(&self) -> CommitCounter { self.inner.commit() }
    pub fn is<S: BevyScene>(&self) -> bool { self.type_id == TypeId::of::<BevyInstance<S>>() }

    pub fn get<S: BevyScene>(&self) -> Option<&BevyInstance<S>> {
        if self.is::<S>() { self.inner.as_any().downcast_ref::<BevyInstance<S>>() } else { None }
    }

    pub fn get_mut<S: BevyScene>(&mut self) -> Option<&mut BevyInstance<S>> {
        if self.is::<S>() { self.inner.as_any_mut().downcast_mut::<BevyInstance<S>>() } else { None }
    }

    #[doc(hidden)] pub fn smithay_id(&self) -> &Id { self.inner.smithay_id() }
    #[doc(hidden)] pub fn tick(&mut self) { self.inner.tick(); }
    #[doc(hidden)] pub fn bump_commit(&mut self) { self.inner.bump_commit(); }

    #[doc(hidden)]
    /// `None` when there is nothing to show yet — the off-thread path has no
    /// buffer until the worker publishes its first frame, and drawing an
    /// unwritten one would flash garbage.
    pub fn element_in(&self, transform: &Transform, output_size: Size<f64, Physical>) -> Option<BevyRenderElement> {
        let dmabuf = self.inner.dmabuf()?;
        let location = self.screen_location(transform, output_size);
        let world_zoom = match self.space { BevySpace::Screen => 1.0, BevySpace::World => transform.zoom };
        Some(BevyRenderElement {
            texture: self.inner.texture_handle().cloned(),
            dmabuf,
            space: self.space,
            location,
            // What the buffer actually holds, which lags a resize the compositor
            // has already applied. Using the requested size would sample past it.
            size: self.inner.rendered_size().unwrap_or_else(|| self.inner.size()),
            world_zoom,
            id: self.inner.smithay_id().clone(), commit_counter: self.inner.commit(),
        })
    }

    #[doc(hidden)]
    pub fn apply_pending_resize(&mut self, render_node: &str, wgpu_ctx: &WgpuVulkanContext, gles: &mut GlesRenderer) -> Result<bool, SurfaceError> {
        self.inner.apply_pending_resize(render_node, wgpu_ctx, gles)
    }

    /// Match the ring to the live setting.
    ///
    /// Gated on the REQUEST, not on a clock — see `depth_failed`. Returns the
    /// error only for a request that has not already been refused, so a wedged
    /// instance logs once instead of once per composite.
    #[doc(hidden)]
    pub fn sync_depth(&mut self, node: &str, ctx: &WgpuVulkanContext, gles: &mut GlesRenderer, want: usize) -> Result<(), SurfaceError> {
        let request = (want, self.inner.size());
        if !self.depth_failed.worth_trying(&request) {
            return Ok(());
        }
        match self.inner.sync_depth(node, ctx, gles, want) {
            Ok(()) => {
                self.depth_failed.succeeded();
                Ok(())
            }
            Err(e) => match self.depth_failed.failed(request) {
                true => Err(e),
                false => Ok(()),
            },
        }
    }
}

impl std::fmt::Debug for BevyItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevyItem")
            .field("handle", &self.handle_id()).field("space", &self.space)
            .field("location", &self.location()).field("size", &self.size()).finish()
    }
}
