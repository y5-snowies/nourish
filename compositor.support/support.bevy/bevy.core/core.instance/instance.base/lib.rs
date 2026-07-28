//! Per-instance bundle: `BevySurface` (output) + `BevyRuntime<S>` + placement.

use std::any::Any;

use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_host_base::BevyRuntime;
use compositor_support_bevy_core_scene_base::BevyScene;
use compositor_support_bevy_core_surface_base::BevySurface;
use compositor_model_debug_instance_record::trace;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Point, Size};

pub struct BevyInstance<S: BevyScene> {
    #[doc(hidden)] pub id: HandleId,
    #[doc(hidden)] pub smithay_id: Id,
    #[doc(hidden)] pub commit: CommitCounter,
    #[doc(hidden)] pub location: Point<i32, Physical>,
    #[doc(hidden)] pub output_surface: BevySurface,
    #[doc(hidden)] pub scale_factor: f32,
    #[doc(hidden)] pub runtime: BevyRuntime<S>,
    #[doc(hidden)] pub pending_resize: Option<(Size<i32, Physical>, f32)>,
}

impl<S: BevyScene> BevyInstance<S> {
    pub fn handle_id(&self) -> HandleId { self.id }
    pub fn location(&self) -> Point<i32, Physical> { self.location }
    pub fn size(&self) -> Size<i32, Physical> { self.output_surface.size }
    pub fn runtime(&self) -> &BevyRuntime<S> { &self.runtime }
    pub fn runtime_mut(&mut self) -> &mut BevyRuntime<S> { &mut self.runtime }
    pub fn scene(&self) -> &S { self.runtime.scene() }
    pub fn scene_mut(&mut self) -> &mut S { self.runtime.scene_mut() }
}

#[doc(hidden)]
pub trait BevyInstanceAny: Any {
    fn handle_id(&self) -> HandleId;
    fn smithay_id(&self) -> &Id;
    fn commit(&self) -> CommitCounter;
    fn bump_commit(&mut self);
    fn location(&self) -> Point<i32, Physical>;
    fn size(&self) -> Size<i32, Physical>;
    fn set_location(&mut self, p: Point<i32, Physical>);
    fn tick(&mut self);
    fn texture_handle(&self) -> &GlesTexture;
    /// Strict read-only accessor for the surface's underlying dmabuf (so Vulkan
    /// can import the bevy output natively; GLES samples the texture above).
    fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf;
    fn apply_pending_resize(&mut self, render_node: &str, wgpu_ctx: &WgpuVulkanContext, gles: &mut GlesRenderer) -> Result<bool, SurfaceError>;
    fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32);
    fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<S: BevyScene> BevyInstanceAny for BevyInstance<S> {
    fn handle_id(&self) -> HandleId { self.id }
    fn smithay_id(&self) -> &Id { &self.smithay_id }
    fn commit(&self) -> CommitCounter { self.commit }
    fn bump_commit(&mut self) { self.commit.increment(); }
    fn location(&self) -> Point<i32, Physical> { self.location }
    fn size(&self) -> Size<i32, Physical> { self.output_surface.size }
    fn set_location(&mut self, p: Point<i32, Physical>) { self.location = p; }

    fn tick(&mut self) {
        self.runtime.update();
        self.commit.increment();
        trace!("tick handle={:?}", self.id);
    }

    fn texture_handle(&self) -> &GlesTexture { &self.output_surface.gles_texture }
    fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        &self.output_surface.allocated.dmabuf
    }

    fn apply_pending_resize(&mut self, render_node: &str, wgpu_ctx: &WgpuVulkanContext, gles: &mut GlesRenderer) -> Result<bool, SurfaceError> {
        let Some((new_size, scale_factor)) = self.pending_resize.take() else { return Ok(false) };
        if new_size == self.output_surface.size && scale_factor == self.scale_factor {
            return Ok(false);
        }
        trace!("resize handle={:?} old={:?} new={new_size:?}", self.id, self.output_surface.size);

        self.output_surface.resize(render_node, wgpu_ctx, gles, new_size)?;
        self.scale_factor = scale_factor;
        self.runtime.resize((new_size.w as u32, new_size.h as u32), self.scale_factor);
        self.commit.increment();
        Ok(true)
    }

    fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32) {
        self.pending_resize = Some((new_size, scale_factor));
    }

    fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)> { self.pending_resize }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
