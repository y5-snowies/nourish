//! `VulkanFramebuffer` (a bind target) and `VulkanFrame` (an in-progress frame).
//!
//! The frame model bridges Smithay's incremental `Frame` API (render → clear →
//! draw* → finish) onto the piece-crates' single-pass `record_composition`
//! helper: each `clear`/`draw_solid`/`render_texture_*` call appends a `DrawOp`,
//! and `finish()` replays them inside one `record_composition` pass, then
//! submits. Foundation simplifications (marked below): the clear is approximated
//! as a full-target clear, and submission is synchronous (`device_wait_idle`).

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_pipeline_composite_base::composite::PushQuad;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{Color32F, ContextId, Frame, Texture};
use smithay::backend::allocator::Fourcc;
use smithay::utils::{Buffer as BufferCoord, Physical, Point, Rectangle, Size, Transform};
use std::marker::PhantomData;

use crate::error::VulkanError;
use crate::renderer::VulkanRenderer;
use crate::texture::VulkanTexture;

/// A render target the renderer can draw into. Owns the color-attachment image
/// imported from the bound dmabuf; destroyed when the framebuffer drops.
pub struct VulkanFramebuffer<'buffer> {
    pub(crate) device: ash::Device,
    pub(crate) image: vk::Image,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
    pub(crate) format: vk::Format,
    pub(crate) fourcc: Option<Fourcc>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) _marker: PhantomData<&'buffer mut ()>,
}

impl std::fmt::Debug for VulkanFramebuffer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanFramebuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl Texture for VulkanFramebuffer<'_> {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn size(&self) -> Size<i32, BufferCoord> {
        Size::from((self.width as i32, self.height as i32))
    }
    fn format(&self) -> Option<Fourcc> {
        self.fourcc
    }
}

impl Drop for VulkanFramebuffer<'_> {
    fn drop(&mut self) {
        unsafe {
            if self.view != vk::ImageView::null() {
                self.device.destroy_image_view(self.view, None);
            }
            if self.image != vk::Image::null() {
                self.device.destroy_image(self.image, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
}

/// One resolved fullscreen-shader variant queued for this frame: the SPIR-V
/// module + entry points (cached as a `FullscreenPass` keyed by `id`) and the
/// owned push-constant bytes. The producing scene element owns all shader
/// specifics; the renderer just runs it.
pub(crate) struct ShaderVariant {
    pub id: u64,
    pub spv: Vec<u8>,
    /// Separate vertex-stage module (set when the fragment was compiled alone,
    /// e.g. a `glsl/` bundle paired with a fullscreen vertex).
    pub vert_spv: Option<Vec<u8>>,
    pub vert_entry: String,
    pub frag_entry: String,
    pub push: Vec<u8>,
}

/// A single queued draw: a solid fill or a textured quad. Order is preserved
/// (back-to-front z-order is the call order from the scene).
pub(crate) enum DrawOp {
    Solid { quad: PushQuad },
    Textured {
        view: vk::ImageView,
        quad: PushQuad,
        /// Per-surface HDR composite flag `[transfer, is_hdr, 0, 0]` (M5).
        surf: [f32; 4],
    },
    /// A fullscreen native shader pass (e.g. the parallax background): the SDR
    /// variant plus an optional HDR-output variant; `submit_frame` builds/caches
    /// a `FullscreenPass` per variant and picks by the active output mode.
    ShaderPass {
        sdr: ShaderVariant,
        hdr: Option<ShaderVariant>,
    },
}

pub struct VulkanFrame<'frame, 'buffer> {
    pub(crate) renderer: &'frame mut VulkanRenderer,
    pub(crate) framebuffer: &'frame mut VulkanFramebuffer<'buffer>,
    pub(crate) output_size: Size<i32, Physical>,
    pub(crate) transform: Transform,
    pub(crate) clear: [f32; 4],
    pub(crate) ops: Vec<DrawOp>,
}

impl VulkanFrame<'_, '_> {
    fn extent(&self) -> (u32, u32) {
        (self.framebuffer.width, self.framebuffer.height)
    }
}

impl Frame for VulkanFrame<'_, '_> {
    type Error = VulkanError;
    type TextureId = VulkanTexture;

    fn context_id(&self) -> ContextId<VulkanTexture> {
        self.renderer.context_id_value()
    }

    fn clear(&mut self, color: Color32F, _at: &[Rectangle<i32, Physical>]) -> Result<(), VulkanError> {
        // Foundation: full-target clear (the per-rect `at` semantics are an
        // optimization to add once damage tracking is exercised on hardware).
        self.clear = [color.r(), color.g(), color.b(), color.a()];
        Ok(())
    }

    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), VulkanError> {
        let out = self.extent();
        let quad = compositor_kernel_vulkan_element_solid_base::solid::quad(
            out,
            (dst.loc.x, dst.loc.y, dst.size.w, dst.size.h),
            [color.r(), color.g(), color.b(), color.a()],
        );
        self.ops.push(DrawOp::Solid { quad });
        Ok(())
    }

    fn render_texture_from_to(
        &mut self,
        texture: &VulkanTexture,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _src_transform: Transform,
        alpha: f32,
    ) -> Result<(), VulkanError> {
        let out = self.extent();
        let tw = texture.width().max(1) as f32;
        let th = texture.height().max(1) as f32;
        let src_uv = (
            (src.loc.x as f32) / tw,
            (src.loc.y as f32) / th,
            (src.size.w as f32) / tw,
            (src.size.h as f32) / th,
        );
        let quad = compositor_kernel_vulkan_element_texture_base::texture::quad(
            out,
            (dst.loc.x, dst.loc.y, dst.size.w, dst.size.h),
            src_uv,
            alpha,
        );
        self.ops.push(DrawOp::Textured {
            view: texture.view(),
            quad,
            surf: texture.surf(),
        });
        Ok(())
    }

    fn transformation(&self) -> Transform {
        self.transform
    }

    fn output_size(&self) -> Size<i32, Physical> {
        self.output_size
    }

    fn wait(&mut self, _sync: &SyncPoint) -> Result<(), VulkanError> {
        // Foundation: submission is synchronous (finish() waits the device), so
        // an explicit cross-frame wait is a no-op. Real acquire-fence waits
        // bridge through vulkan.sync once async submission lands.
        Ok(())
    }

    fn finish(self) -> Result<SyncPoint, VulkanError> {
        let extent = self.extent();
        self.renderer.submit_frame(
            self.framebuffer.image,
            self.framebuffer.view,
            self.framebuffer.format,
            extent,
            self.clear,
            self.ops,
        )
    }
}
