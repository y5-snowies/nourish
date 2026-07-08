//! `Bind<Dmabuf>` (the scanout/output target), the exportable output target
//! (`create_output_target`), and the core trait surface (`RendererSuper` /
//! `Renderer`).

use ash::vk;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{
    Bind, ContextId, DebugFlags, ImportDma, Renderer, RendererSuper, TextureFilter,
};
use smithay::utils::{Physical, Size, Transform};
use std::marker::PhantomData;

use crate::error::VulkanError;
use crate::frame::{VulkanFrame, VulkanFramebuffer};
use crate::texture::VulkanTexture;
use super::VulkanRenderer;

impl VulkanRenderer {
    /// Import a dmabuf as a render/transfer target (delegates to
    /// `vulkan.memory/memory.target`).
    pub(super) fn import_dmabuf_as_target(
        &self,
        dmabuf: &Dmabuf,
        usage: vk::ImageUsageFlags,
        make_view: bool,
    ) -> Result<
        (
            vk::Image,
            vk::DeviceMemory,
            Option<vk::ImageView>,
            vk::Format,
            u32,
            u32,
        ),
        VulkanError,
    > {
        compositor_kernel_vulkan_memory_target_base::target::import_target(&self.dev, dmabuf, usage, make_view)
    }

    /// Allocate an exportable `size`-sized color target and export it as a
    /// dmabuf. The dmabuf can be bound as a render target (`Bind<Dmabuf>`) and
    /// also imported by another renderer (e.g. winit's GLES) for presentation.
    pub fn create_output_target(
        &self,
        size: (i32, i32),
    ) -> Result<Dmabuf, VulkanError> {
        let fourcc = Fourcc::Argb8888;
        let vk_fmt = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc)
            .ok_or(VulkanError::UnsupportedFormat(fourcc))?;
        let mods: Vec<_> =
            compositor_kernel_vulkan_format_modifier_base::modifier::modifiers(&self.phd, vk_fmt)
                .into_iter()
                .map(|(m, _)| m)
                .collect();
        let target = compositor_kernel_vulkan_memory_export_base::export::create_exportable(
            &self.dev,
            &self.phd,
            fourcc,
            (size.0 as u32, size.1 as u32),
            &mods,
        )
        .map_err(|e| VulkanError::Vk(format!("create_exportable: {e:?}")))?;
        let dmabuf = compositor_kernel_vulkan_memory_export_base::export::export(&self.dev, &target)
            .map_err(|e| VulkanError::Vk(format!("export: {e:?}")))?;
        // The exported dmabuf is a standalone kernel object (its fd is a dup of
        // the memory), and `bind()` re-imports the dmabuf as its own
        // COLOR_ATTACHMENT image each frame — so this source VkImage/memory/view
        // is never used after export. Destroying it here fixes a per-resize leak
        // of a full-screen image + dedicated memory (the previous code dropped
        // the `ExportableImage` value, leaking the GPU objects it named).
        target.destroy(&self.dev);
        Ok(dmabuf)
    }
}

impl RendererSuper for VulkanRenderer {
    type Error = VulkanError;
    type TextureId = VulkanTexture;
    type Framebuffer<'buffer> = VulkanFramebuffer<'buffer>;
    type Frame<'frame, 'buffer>
        = VulkanFrame<'frame, 'buffer>
    where
        'buffer: 'frame,
        Self: 'frame;
}

impl Renderer for VulkanRenderer {
    fn context_id(&self) -> ContextId<VulkanTexture> {
        self.context_id.clone()
    }

    fn downscale_filter(&mut self, filter: TextureFilter) -> Result<(), VulkanError> {
        self.downscale = filter;
        Ok(())
    }

    fn upscale_filter(&mut self, filter: TextureFilter) -> Result<(), VulkanError> {
        self.upscale = filter;
        Ok(())
    }

    fn set_debug_flags(&mut self, flags: DebugFlags) {
        self.debug_flags = flags;
    }

    fn debug_flags(&self) -> DebugFlags {
        self.debug_flags
    }

    fn render<'frame, 'buffer>(
        &'frame mut self,
        framebuffer: &'frame mut VulkanFramebuffer<'buffer>,
        output_size: Size<i32, Physical>,
        dst_transform: Transform,
    ) -> Result<VulkanFrame<'frame, 'buffer>, VulkanError>
    where
        'buffer: 'frame,
    {
        Ok(VulkanFrame {
            renderer: self,
            framebuffer,
            output_size,
            transform: dst_transform,
            clear: [0.0, 0.0, 0.0, 0.0],
            ops: Vec::new(),
            current_meta: compositor_orchestration_draw_dispatch_frame::ElementMeta::SCREEN,
        })
    }

    fn wait(&mut self, _sync: &SyncPoint) -> Result<(), VulkanError> {
        // Synchronous foundation; nothing to wait on.
        Ok(())
    }
}

impl Bind<Dmabuf> for VulkanRenderer {
    fn bind<'a>(
        &mut self,
        target: &'a mut Dmabuf,
    ) -> Result<VulkanFramebuffer<'a>, VulkanError> {
        let (image, memory, view, format, width, height) =
            self.import_dmabuf_as_target(target, vk::ImageUsageFlags::COLOR_ATTACHMENT, true)?;
        Ok(VulkanFramebuffer {
            device: self.dev.device.clone(),
            image,
            memory,
            view: view.expect("make_view=true ⇒ Some(view)"),
            format,
            fourcc: Some(target.format().code),
            width,
            height,
            _marker: PhantomData,
        })
    }

    fn supported_formats(&self) -> Option<smithay::backend::allocator::format::FormatSet> {
        Some(self.dmabuf_formats())
    }
}
