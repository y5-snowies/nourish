//! The `VulkanRenderer` itself and its Smithay trait implementations.
//!
//! Split across submodules so no single file is unwieldy:
//! - [`lifecycle`]: construction (`new`/`new_default`/`validate`) + the one-time
//!   device-object creators.
//! - [`pipelines`]: the per-format composite/background/HDR pipeline caches.
//! - [`submit`]: `submit_frame` (SDR + HDR record + sync) and the post-scene
//!   capture handoff.
//! - [`import`]: the `Import*` trait family (dmabuf / SHM / mem), delegating the
//!   GPU work to the `memory.*` piece-crates and reusing textures via the SHM
//!   cache.
//! - [`bind`]: `Bind<Dmabuf>` + the exportable output target + the trait
//!   surface (`RendererSuper`/`Renderer`).

mod bind;
mod import;
mod lifecycle;
mod mipgen;
mod pipelines;
mod submit;

use ash::vk;
use compositor_kernel_vulkan_capture_blit_base::blit::CaptureCache;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_device_queue_base::queue::RenderQueue;
use compositor_kernel_vulkan_memory_upload_base::upload::StagingBuffer;
use compositor_kernel_vulkan_pipeline_composite_base::composite::{AaComposite, CompositePipelines};
use compositor_kernel_vulkan_pipeline_fullscreen_base::fullscreen::FullscreenPass;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::{ContextId, DebugFlags, TextureFilter};
use smithay::backend::vulkan::PhysicalDevice;
use std::collections::HashMap;
use compositor_developer_stats_registry_base::base as stats;

use crate::texture::VulkanTexture;

/// A Vulkan renderer implementing Smithay's `Renderer`/`Frame` family.
///
/// Execution model (foundation): one reused command buffer, synchronous
/// submission (`device_wait_idle` per frame) by default, one composite pipeline
/// set per target format, a per-frame-reset descriptor pool.
pub struct VulkanRenderer {
    pub(super) dev: VulkanDevice,
    pub(super) phd: PhysicalDevice,
    pub(super) queue: RenderQueue,
    pub(super) command_pool: vk::CommandPool,
    pub(super) cmd: vk::CommandBuffer,
    pub(super) pipeline_cache: vk::PipelineCache,
    pub(super) pipelines: HashMap<vk::Format, CompositePipelines>,
    /// Per-format world anti-aliasing pipelines (windows + iced AA). Built on
    /// demand only when the live `AA_MODE` is non-Off; the plain `pipelines`
    /// above stay the untouched default path. See `pipeline.composite::aa`.
    pub(super) aa_pipelines: HashMap<vk::Format, AaComposite>,
    /// Reusable per-surface mipped copies for the trilinear/aniso AA modes
    /// (`RefCell` so the per-frame acquire + record borrow cleanly alongside the
    /// other renderer field borrows in `submit`). Empty until such a mode runs.
    pub(super) mipgen: std::cell::RefCell<mipgen::MipGen>,
    /// Whether AA was active last frame — drives lazy build on activation and
    /// resource teardown on deactivation (see `teardown_aa`).
    pub(super) aa_was_active: bool,
    /// Generic native fullscreen-shader passes, keyed by `(shader id, format)`.
    /// Built on demand from the scene's `DrawOp::ShaderPass` and held for the
    /// renderer's lifetime. The parallax background (SDR + HDR variants) is one
    /// such pass; the kernel keeps no shader-specific knowledge.
    pub(super) shader_passes: HashMap<(u64, vk::Format), FullscreenPass>,
    /// Per-format HDR composite pipelines (M5 1a), created on demand only when
    /// the HDR path is active. The SDR `pipelines` above are untouched.
    pub(super) hdr_pipelines: HashMap<vk::Format, crate::hdr_composite::HdrComposite>,
    /// HDR output path active (COMPOSITOR_HDR + capable display); set from the
    /// backend. When true `submit_frame` composites via `hdr_pipelines` and
    /// outputs PQ/BT.2020.
    pub(super) hdr_enabled: bool,
    pub(super) descriptor_pool: vk::DescriptorPool,
    pub(super) timeline: vk::Semaphore,
    /// Binary, SYNC_FD-exportable semaphore signaled by each render submit; the
    /// native KMS path exports it directly as a `sync_file` for the atomic-commit
    /// IN_FENCE.
    pub(super) render_semaphore: vk::Semaphore,
    /// VkFence signaled by each native-path submit — CPU pacing for the reused
    /// command buffer. Created signaled.
    pub(super) frame_fence: vk::Fence,
    /// The display's DRM device fd, set on the native backend (None under
    /// winit). Its presence selects the native KMS IN_FENCE path.
    pub(super) drm_fd: Option<smithay::backend::drm::DrmDeviceFd>,
    /// Opt in to the native KMS IN_FENCE path via `COMPOSITOR_RENDERER_SYNC=infence`.
    /// DEFAULT IS OFF (synchronous `device_wait_idle` submit).
    pub(super) native_fence_optin: bool,
    /// Throttle for the per-frame native-fence-export warning (once/min).
    pub(super) last_fence_warn: Option<std::time::Instant>,
    /// Post-scene capture targets for THIS frame: the registry's entry dmabufs to
    /// copy the composed scene into. Set by the backend before `render_frame`;
    /// consumed (and cleared) in `submit_frame`.
    pub(super) capture_targets:
        Vec<(Dmabuf, Option<smithay::utils::Rectangle<i32, smithay::utils::Physical>>)>,
    /// Capture-target dmabufs imported as TRANSFER_DST images, re-imported only
    /// when the target set changes (the leak fix lives in `capture.blit`).
    pub(super) capture_cache: CaptureCache,
    /// Reusable host-visible staging buffer for SHM uploads (grows on demand),
    /// so steady-state SHM updates allocate no new host memory.
    pub(super) shm_staging: StagingBuffer,
    pub(super) frame_counter: u64,
    pub(super) debug_flags: DebugFlags,
    pub(super) downscale: TextureFilter,
    pub(super) upscale: TextureFilter,
    pub(super) context_id: ContextId<VulkanTexture>,
}

impl std::fmt::Debug for VulkanRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanRenderer")
            .field("queue_family", &self.queue.family_index)
            .field("frame_counter", &self.frame_counter)
            .finish()
    }
}

impl VulkanRenderer {
    pub(crate) fn context_id_value(&self) -> ContextId<VulkanTexture> {
        self.context_id.clone()
    }

    /// Provide the display's DRM device fd (native backend). With the IN_FENCE
    /// opt-in (`COMPOSITOR_RENDERER_SYNC=infence`) its presence switches
    /// `finish()` to the KMS IN_FENCE path; otherwise the default synchronous
    /// submit is kept.
    pub fn set_drm_fd(&mut self, fd: smithay::backend::drm::DrmDeviceFd) {
        self.drm_fd = Some(fd);
        if self.native_fence_optin {
            stats::set_sync_mode("native KMS IN_FENCE (sync_file)");
        }
    }

    /// Hand the renderer this frame's post-scene capture targets (the capture
    /// registry's entry dmabufs, each with an optional source sub-rect for
    /// region captures — `None` means copy the whole composed scene). The next
    /// `submit_frame` copies into each, then clears the list. Empty (the
    /// default) ⇒ no capture.
    pub fn set_capture_targets(
        &mut self,
        targets: Vec<(Dmabuf, Option<smithay::utils::Rectangle<i32, smithay::utils::Physical>>)>,
    ) {
        self.capture_targets = targets;
    }

    /// Enable/disable the HDR output path (M5). When on, `submit_frame`
    /// composites via the WGSL HDR pipeline (PQ/BT.2020 + live tuning) instead
    /// of the SDR composite. Set from the backend when COMPOSITOR_HDR is active.
    pub fn set_hdr_enabled(&mut self, on: bool) {
        self.hdr_enabled = on;
    }

    pub(crate) fn use_hdr(&self) -> bool {
        self.hdr_enabled
    }

    /// True when the native KMS IN_FENCE path should be used: explicitly opted in
    /// (`COMPOSITOR_RENDERER_SYNC=infence`) and a DRM fd is present (native).
    pub(super) fn use_native_fence(&self) -> bool {
        self.native_fence_optin && self.drm_fd.is_some()
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.dev.device.device_wait_idle();
            // Capture-target imports + the reusable SHM staging buffer.
            self.capture_cache.destroy(&self.dev);
            self.shm_staging.destroy(&self.dev);
            let passes: Vec<_> = self.shader_passes.drain().collect();
            for (_, p) in passes {
                p.destroy(&self.dev);
            }
            let hdr: Vec<_> = self.hdr_pipelines.drain().collect();
            for (_, h) in hdr {
                h.destroy(&self.dev);
            }
            let aa: Vec<_> = self.aa_pipelines.drain().collect();
            for (_, a) in aa {
                a.destroy(&self.dev);
            }
            self.mipgen.borrow_mut().destroy(&self.dev);
            for (_, p) in self.pipelines.drain() {
                self.dev.device.destroy_pipeline(p.textured, None);
                self.dev.device.destroy_pipeline(p.solid, None);
                self.dev.device.destroy_pipeline_layout(p.layout, None);
                self.dev
                    .device
                    .destroy_descriptor_set_layout(p.descriptor_layout, None);
                self.dev.device.destroy_sampler(p.sampler, None);
            }
            self.dev.device.destroy_descriptor_pool(self.descriptor_pool, None);
            self.dev.device.destroy_semaphore(self.timeline, None);
            self.dev.device.destroy_semaphore(self.render_semaphore, None);
            self.dev.device.destroy_fence(self.frame_fence, None);
            self.dev
                .device
                .destroy_pipeline_cache(self.pipeline_cache, None);
            self.dev.device.destroy_command_pool(self.command_pool, None);
            // Destroy the logical device LAST (after all device-child objects,
            // before the owning instance drops).
            self.dev.device.destroy_device(None);
        }
    }
}
