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
        // DEVICE-LOCAL: this target is written and sampled by THIS Vulkan device,
        // so no cross-API narrowing applies and the device's full modifier list
        // below is legal. See `Answer::DeviceLocal`.
        let fourcc = compositor_kernel_graphic_format_answer_base::answer::constant(compositor_kernel_graphic_format_answer_base::answer::Consumer::VulkanOutputTarget).0;
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
            clear_rects: Vec::new(),
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
        use crate::frame::TargetAcquire;

        let weak = target.weak();
        // TRANSFER_SRC only when something will actually blit FROM this target.
        //
        // `capture.blit` is the only such consumer — it takes the composed scene as the
        // `srcImage` of `vkCmdBlitImage`, which the spec requires TRANSFER_SRC_BIT for
        // (VUID-vkCmdBlitImage-srcImage-00219, observed firing). `mipgen` does NOT: it
        // blits from its own `MipChain` image, which it creates itself.
        //
        // `set_capture_targets` runs before `render_frame` (and so before this), so the
        // frame's answer is already known here. Lazily UPGRADE and never downgrade: the
        // first capture re-imports the target with the extra bit and it keeps it, because
        // a screenshot is one frame and toggling the flag per frame would otherwise
        // recreate the scanout image constantly.
        let want_src = !self.capture_targets.is_empty();
        // Reuse the import rather than paying create-image + dedicated
        // import-alloc + view — and the matching destroy — on every frame. A
        // scanout swapchain cycles a handful of buffers forever, so after the
        // first pass over them this is always a hit.
        if let Some(t) = self.target_cache.get(&weak).filter(|t| t.transfer_src || !want_src) {
            return Ok(VulkanFramebuffer {
                device: self.dev.device.clone(),
                retire: self.retired.clone(),
                image: t.image,
                memory: t.memory,
                view: t.view,
                format: t.format,
                fourcc: Some(target.format().code),
                width: t.width,
                height: t.height,
                owned: false,
                // The same VkImage the last composite left in GENERAL, so its
                // contents (and layout) can be acquired back rather than dropped.
                acquire: TargetAcquire::Cached,
                _marker: PhantomData,
            });
        }

        // A cache entry that exists but lacks TRANSFER_SRC (capture armed after it was
        // minted) is retired here rather than leaked — the frames that used it are the
        // ones already drained.
        if let Some(old) = self.target_cache.remove(&weak) {
            Self::destroy_target(&self.dev, &old);
        }
        let mut usage = vk::ImageUsageFlags::COLOR_ATTACHMENT;
        if want_src {
            usage |= vk::ImageUsageFlags::TRANSFER_SRC;
        }
        let (image, memory, view, format, width, height) =
            self.import_dmabuf_as_target(target, usage, true)?;
        let view = view.expect("make_view=true ⇒ Some(view)");
        self.target_cache.insert(
            weak,
            crate::renderer::CachedTarget {
                transfer_src: want_src, image, memory, view, format, width, height,
            },
        );
        Ok(VulkanFramebuffer {
            device: self.dev.device.clone(),
            retire: self.retired.clone(),
            image,
            memory,
            view,
            format,
            fourcc: Some(target.format().code),
            width,
            height,
            // The cache owns it from here; the framebuffer must not retire it.
            owned: false,
            // A brand-new VkImage over this dmabuf: nothing of ours is in it that
            // this VkImage could preserve, so discard and clear in full. Cache
            // membership IS the "have we composited into this buffer" record —
            // entries are reaped only once the dmabuf itself is gone, so a miss
            // always means a genuinely untouched buffer.
            acquire: TargetAcquire::Fresh,
            _marker: PhantomData,
        })
    }

    fn supported_formats(&self) -> Option<smithay::backend::allocator::format::FormatSet> {
        Some(self.dmabuf_formats())
    }
}
