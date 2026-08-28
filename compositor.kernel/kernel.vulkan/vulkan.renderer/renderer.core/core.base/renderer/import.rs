//! The `Import*` trait family: dmabuf (sampled client buffers), SHM/mem (CPU
//! upload), and the EGL stub. GPU work is delegated to the `memory.*`
//! piece-crates; this module wraps the results into `VulkanTexture`s.

use ash::vk;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::backend::renderer::{ImportDma, ImportDmaWl, ImportMem, ImportMemWl};
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};
use std::sync::Arc;

use crate::error::VulkanError;
use crate::texture::{TextureInner, VulkanTexture};
use super::VulkanRenderer;

impl VulkanRenderer {
    /// One-shot layout transition for a freshly imported sampled image
    /// (`UNDEFINED → SHADER_READ_ONLY_OPTIMAL`).
    pub(super) fn transition_to_sampled(&self, image: vk::Image) -> Result<(), VulkanError> {
        let cmd = Self::alloc_command_buffer(&self.dev, self.command_pool)?;
        let dev = &self.dev.device;
        unsafe {
            dev.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            // When the device has multi-plane support, acquire the imported dmabuf from its
            // external producer (VK_QUEUE_FAMILY_FOREIGN_EXT) so the driver
            // interprets the existing DRM-modifier / CCS-compressed contents
            // rather than reinitializing them; oldLayout = UNDEFINED is the
            // content-preserving pairing for a foreign acquire. Without it both
            // indices stay IGNORED (the original no-transfer barrier).
            let mut barrier = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            if self.dev.multiplane {
                barrier = barrier
                    .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                    .dst_queue_family_index(self.dev.queue_family_index);
            }
            let barriers = [barrier];
            let dep = vk::DependencyInfo::default().image_memory_barriers(&barriers);
            dev.cmd_pipeline_barrier2(cmd, &dep);
            dev.end_command_buffer(cmd)?;
            let cmds = [cmd];
            // Fence-scoped wait: this fires on every NEW dmabuf import, and a
            // device_wait_idle would also drain the in-flight composite on the
            // native IN_FENCE path.
            let fence = dev.create_fence(&vk::FenceCreateInfo::default(), None)?;
            let submit = vk::SubmitInfo::default().command_buffers(&cmds);
            let result = dev
                .queue_submit(self.queue.queue, &[submit], fence)
                .and_then(|()| dev.wait_for_fences(&[fence], true, u64::MAX));
            dev.destroy_fence(fence, None);
            dev.free_command_buffers(self.command_pool, &cmds);
            result?;
        }
        Ok(())
    }

    /// Re-acquire every cache-served dmabuf from its external producer for this
    /// frame's sampling. Recorded into the frame's own command buffer — no extra
    /// submit and no host wait, which is the whole point of the import cache —
    /// and batched into ONE barrier covering every image, so the cost is O(1)
    /// API calls regardless of how many surfaces are on screen.
    ///
    /// This carries the queue-family ACQUIRE half of `transition_to_sampled`, and
    /// it has to: these dmabufs are written by a producer on a device we do not
    /// own (bevy and iced each hold their own `wgpu` Vulkan device), so every
    /// frame their contents arrive from `VK_QUEUE_FAMILY_FOREIGN_EXT` again.
    /// Acquiring is what makes the driver interpret the producer's DRM-modifier /
    /// CCS-compressed layout rather than reading it as if we had written it; a
    /// plain same-family barrier makes the writes *available* but skips that
    /// re-interpretation, which a continuously redrawn surface notices.
    ///
    /// It does NOT copy that function's `oldLayout = UNDEFINED`, and the
    /// difference matters. `UNDEFINED` is not "unknown" — it is permission for
    /// the implementation to DISCARD the range's contents. At first import that
    /// is both unavoidable (a fresh `VkImage` must start `UNDEFINED`) and
    /// harmless (the image object has no contents of its own to lose). Here the
    /// image is long-lived and the driver still tracks it as
    /// `SHADER_READ_ONLY_OPTIMAL`, so naming that layout instead keeps the
    /// acquire content-PRESERVING — the spec guarantees preservation for an
    /// ownership acquire whose `oldLayout` is not `UNDEFINED` — and volunteers no
    /// discard licence on pixels we are about to sample. There is no matching
    /// release to stay consistent with either way: the foreign party is another
    /// device that never performs Vulkan ownership transfers on our behalf.
    pub(super) fn record_pending_acquires(&self, cmd: vk::CommandBuffer, textures: &[VulkanTexture]) {
        if textures.is_empty() {
            return;
        }
        let barriers: Vec<vk::ImageMemoryBarrier2> = textures
            .iter()
            .map(|t| &t.inner.image)
            .map(|image| {
                let mut barrier = vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(*image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                if self.dev.multiplane {
                    barrier = barrier
                        .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                        .dst_queue_family_index(self.dev.queue_family_index);
                }
                barrier
            })
            .collect();
        unsafe {
            self.dev.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&barriers),
            );
        }
    }
}

impl ImportDma for VulkanRenderer {
    /// EXACTLY what `import_dmabuf` will accept — no hand-kept list.
    ///
    /// Both sides now derive from `format.table`, which is exhaustive over
    /// `Fourcc`: this walks the table's importable formats and keeps what the
    /// device can sample through which modifier, and `import_dmabuf` decides by
    /// classifying the same table. There is no way for the two to drift, which
    /// is what a hand-written list here could not promise — it went stale twice
    /// (10-bit, then fp16), and each time the gap surfaced as a blank window.
    fn dmabuf_formats(&self) -> smithay::backend::allocator::format::FormatSet {
        compositor_kernel_vulkan_format_modifier_base::modifier::import_formats(&self.phd)
    }

    fn import_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        _damage: Option<&[Rectangle<i32, BufferCoord>]>,
    ) -> Result<VulkanTexture, VulkanError> {
        // POLICY, ahead of capability. `vk_format` answers what this device can
        // sample; it says yes to the float formats, which the SDR composite cannot
        // EXPRESS (linear-light values read as if sRGB-encoded). Those are already
        // withheld from the advertisement and refused at buffer creation, so a client
        // cannot reach here — but a v3/wl_drm client, XWayland, or one of our own
        // producers still could, and then the only sign would be a too-dark window.
        // Refusing here is what makes "not advertised" and "not importable" the same
        // statement rather than two that can drift.
        let code = dmabuf.format().code;
        if !compositor_kernel_graphic_format_resolve_base::resolve::may_import(&self.formats, code) {
            return Err(VulkanError::UnsupportedFormat(code));
        }
        // Reuse the import for a dmabuf we already hold. The scene lowers
        // iced/bevy nodes by calling this EVERY FRAME for EVERY surface
        // (`draw.node`'s `import_texture`) — unlike client buffers, which smithay
        // caches per commit in `RendererSurfaceState`. Uncached, each of those
        // calls is a create-image + dedicated dmabuf import-alloc + view, plus the
        // fenced layout submit below, plus the matching destroy, so the cost
        // scaled with the number of iced surfaces on screen. The dmabuf backing a
        // surface is stable for its lifetime — reallocated only on resize or on
        // the visibility sweep's release, both of which mint a new `Arc` — so
        // pointer identity is a sound key, and a stale entry cannot alias a new
        // buffer.
        let key = dmabuf.weak();
        if let Some(tex) = self.import_cache.get(&key) {
            // Skipping `transition_to_sampled` skips its queue submit AND its host
            // fence wait, which is the actual win: that wait sat on the same queue
            // as the composite, so it drained the in-flight frame once per surface.
            // What it must NOT skip is the barrier itself — the producer redraws
            // into this dmabuf every frame, so its contents keep arriving from a
            // foreign queue family and have to be acquired again each time. The
            // frame that samples it re-records that same acquire, batched, into
            // its own command buffer (see `pending_acquires` /
            // `record_pending_acquires`) — same barrier, no submit, no host wait.
            // Deduped: a surface visible in several panes imports once per pane,
            // and one barrier per image covers them all.
            let tex = tex.clone();
            if !self
                .pending_acquires
                .iter()
                .any(|t| t.inner.image == tex.inner.image)
            {
                self.pending_acquires.push(tex.clone());
            }
            return Ok(tex);
        }
        let imported =
            compositor_kernel_vulkan_memory_import_base::import::import(&self.dev, &self.phd, dmabuf)
                .map_err(|e| VulkanError::Import(e.to_string()))?;
        self.transition_to_sampled(imported.image)?;
        let texture = VulkanTexture {
            inner: Arc::new(TextureInner {
                device: self.dev.device.clone(),
                image: imported.image,
                memory: imported.memory,
                extra_memory: imported.extra_memory,
                view: imported.view,
                format: imported.format,
                fourcc: Some(dmabuf.format().code),
                width: imported.size.0,
                height: imported.size.1,
                owns_memory: true,
                retire: Some(self.retired_textures.clone()),
            }),
            surf: [0.0; 4],
            // Keep a WEAK handle on the client's own dmabuf: this image's memory
            // was IMPORTED, not allocated exportable, so it cannot be re-exported —
            // but the original fd can be handed to a second device, which is how
            // the background worker gets at window content.
            //
            // Weak, not strong: this texture is about to be stored in
            // `import_cache`, which is KEYED by the weak form of this same dmabuf
            // and evicted by upgrading that key. A strong clone here would make
            // every entry keep its own key alive, so nothing would ever be reaped.
            source: Some(dmabuf.weak()),
            shared: None,
        };
        self.import_cache.insert(key, texture.clone());
        Ok(texture)
    }
}

// Wayland dmabuf import uses the provided default (get_dmabuf + import_dmabuf).
impl ImportDmaWl for VulkanRenderer {
    fn import_dma_buffer(
        &mut self,
        buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
        surface: Option<&smithay::wayland::compositor::SurfaceData>,
        damage: &[Rectangle<i32, BufferCoord>],
    ) -> Result<VulkanTexture, VulkanError> {
        let dmabuf = smithay::wayland::dmabuf::get_dmabuf(buffer)
            .map_err(|e| VulkanError::Import(format!("get_dmabuf: {e}")))?;
        let mut tex = ImportDma::import_dmabuf(self, dmabuf, Some(damage))?;
        if self.hdr_enabled {
            if let Some(s) = surface {
                tex.set_surf(crate::shm_cache::surf_from_surface(s));
            }
        }
        Ok(tex)
    }
}

/// Whether SHM uploads must be shareable with a second device right now.
///
/// LIVE, not settled at startup: the selected bundle changes whenever the user
/// picks a shader. Read from the same slot every other requirement gate reads, so
/// there is one answer.
///
/// A change is not stranded. `shm_cache` refuses to reuse a cached texture whose
/// shareability no longer matches this, so each surface reallocates correctly on
/// its next commit — in both directions. A surface that never commits again keeps
/// its old image and reports `Source::Unavailable`, which holds the bundle inline
/// and now SAYS so (`draw.offload`), rather than sampling a descriptor it does not
/// own.


impl ImportMem for VulkanRenderer {
    fn import_memory(
        &mut self,
        data: &[u8],
        format: Fourcc,
        size: Size<i32, BufferCoord>,
        _flipped: bool,
    ) -> Result<VulkanTexture, VulkanError> {
        // Hoisted: the upload below takes `&mut self.shm_staging`, so the answer
        // cannot be read through `&self` inside the same call.
        let shared = self.wants_shared_shm();
        let up = compositor_kernel_vulkan_memory_upload_base::upload::create_and_upload(
            &self.dev,
            &self.phd,
            self.command_pool,
            self.queue.queue,
            &mut self.shm_staging,
            data,
            format,
            size,
            shared,
        )?;
        Ok(VulkanTexture {
            inner: Arc::new(TextureInner {
                device: self.dev.device.clone(),
                image: up.image,
                memory: up.memory,
                extra_memory: Vec::new(),
                view: up.view,
                format: up.format,
                fourcc: Some(format),
                width: up.width,
                height: up.height,
                owns_memory: true,
                retire: Some(self.retired_textures.clone()),
            }),
            surf: [0.0; 4],
            source: None,
            // SHM has no client fd, so share the image we uploaded into. Exported
            // ONCE here rather than per frame: the allocation is exportable, and
            // the fd is what the worker duplicates on import. A failure is not
            // fatal — the surface simply cannot be sampled off-thread.
            //
            // Only when the ACTIVE bundle requires window textures — this mints an
            // `OwnedFd` per surface, and the allocation it exports is only
            // exportable under the same answer (`memory.upload`). With no such
            // bundle it is skipped entirely, so a desktop full of SHM clients holds
            // no extra fds.
            shared: shared
                .then(|| {
                    compositor_kernel_vulkan_memory_external_base::external::export(
                        &self.dev, up.memory, up.size, up.format, up.width, up.height,
                    )
                    .ok()
                    .map(std::sync::Arc::new)
                })
                .flatten(),
        })
    }

    fn update_memory(
        &mut self,
        texture: &VulkanTexture,
        data: &[u8],
        region: Rectangle<i32, BufferCoord>,
    ) -> Result<(), VulkanError> {
        // In-place re-upload of the damaged region into the existing image
        // (reuses the device-local allocation rather than allocating a new one).
        compositor_kernel_vulkan_memory_upload_base::upload::update_region(
            &self.dev,
            &self.phd,
            self.command_pool,
            self.queue.queue,
            &mut self.shm_staging,
            texture.inner.image,
            (texture.inner.width, texture.inner.height),
            data,
            region,
        )
    }

    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        Box::new(compositor_kernel_graphic_format_answer_base::answer::shm().iter().copied())
    }
}

impl ImportMemWl for VulkanRenderer {
    fn import_shm_buffer(
        &mut self,
        buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
        surface: Option<&smithay::wayland::compositor::SurfaceData>,
        damage: &[Rectangle<i32, BufferCoord>],
    ) -> Result<VulkanTexture, VulkanError> {
        crate::shm_cache::import_shm_buffer(self, buffer, surface, damage)
    }
}

// Required so `VulkanRenderer: ImportAll` (smithay's EGL-featured blanket impl
// needs ImportEgl). wl_drm/EGL client buffers are legacy; modern clients use
// dmabuf (implemented) or SHM (implemented). egl_reader returns None, so the
// ImportAll dispatcher never routes EGL buffers here in practice.
impl smithay::backend::renderer::ImportEgl for VulkanRenderer {
    fn bind_wl_display(
        &mut self,
        _display: &smithay::reexports::wayland_server::DisplayHandle,
    ) -> Result<(), smithay::backend::egl::Error> {
        Ok(())
    }

    fn unbind_wl_display(&mut self) {}

    fn egl_reader(&self) -> Option<&smithay::backend::egl::display::EGLBufferReader> {
        None
    }

    fn import_egl_buffer(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
        _surface: Option<&smithay::wayland::compositor::SurfaceData>,
        _damage: &[Rectangle<i32, BufferCoord>],
    ) -> Result<VulkanTexture, VulkanError> {
        Err(VulkanError::Unimplemented("import_egl_buffer (wl_drm/EGL)"))
    }
}
