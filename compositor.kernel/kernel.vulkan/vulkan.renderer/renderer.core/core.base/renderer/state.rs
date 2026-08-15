//! The `VulkanRenderer` struct itself, its per-output resources and the
//! inherent methods over them. Re-exported from `renderer`.

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
use compositor_model_stats_registry_base::base as stats;

use crate::texture::VulkanTexture;

/// One output dmabuf imported as a colour attachment and KEPT, rather than
/// re-imported and destroyed every frame. Keyed by the dmabuf's pointer
/// identity, so a swapchain buffer reappearing next frame reuses its `VkImage` —
/// which is also what lets the composite preserve the undamaged remainder,
/// since the driver still tracks that image's layout.
pub(crate) struct CachedTarget {
    pub(crate) image: vk::Image,
    pub(crate) memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
    pub(crate) format: vk::Format,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// A Vulkan renderer implementing Smithay's `Renderer`/`Frame` family.
///
/// Execution model (foundation): one reused command buffer, synchronous
/// submission (`device_wait_idle` per frame) by default, one composite pipeline
/// set per target format, a per-frame-reset descriptor pool.
/// Everything the renderer owns PER OUTPUT.
///
/// Every field here is either sized from the pass's `extent` or filled from the
/// pass's own composite, and `submit_frame` runs once per output. Held as a
/// single instance each — which is what they were — they are shared by every
/// monitor, and the two ways that shows up are both severe:
///
/// * `graph` rebuilds whenever `extent` changes (`GraphExec::prepare`), so two
///   monitors of different sizes destroyed and recreated every intermediate
///   target TWICE PER FRAME. A four-target bundle went from 500 fps to 19.
/// * `offscreen` owns `content` AND `history`. Monitor A composited into
///   `content` and copied it to `history`; monitor B then sampled that same
///   `history` — A's picture. A motion-blur pass blended two different images
///   instead of successive frames of one, alternating and dimming toward the
///   average.
///
/// Keyed by OUTPUT and not by extent: two monitors at the same resolution would
/// collide, which is the common setup and the one that reported the bug.
#[derive(Default)]
pub(crate) struct OutputResources {
    /// Multipass background executor: intermediate targets + per-pass pipelines,
    /// all sized from this output's extent.
    pub(crate) graph: compositor_pipeline_execute_graph_base::graph::GraphExec,
    /// The offscreen `content`/`history`/`windows` path for this output.
    pub(crate) offscreen:
        Option<compositor_pipeline_execute_offscreen_base::offscreen::ContentPath>,
    /// This output's GPU warp-map producer, for a bundle declaring
    /// `evaluate: "map"`.
    pub(crate) warpmap: Option<compositor_pipeline_execute_warpmap_base::base::WarpMap>,
    /// Whether `content` was shared last frame, so a withdrawal happens once.
    pub(crate) shared_content: bool,
    /// This output's composited band and window layer, exported for the worker.
    pub(crate) content_shared:
        Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    pub(crate) windows_shared:
        Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    /// The warp grid this output produced, for the frame driver to hand on.
    pub(crate) warp_grid: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    /// The drawables collected for this output, for the same hop.
    pub(crate) world_set:
        Option<std::sync::Arc<compositor_pipeline_abi_worldset_base::base::WorldSet>>,
}

impl OutputResources {
    fn destroy(mut self, dev: &VulkanDevice) {
        if let Some(w) = self.warpmap {
            w.destroy(dev);
        }
        if let Some(off) = self.offscreen {
            off.destroy(dev);
        }
        self.graph.destroy(dev);
    }
}

pub struct VulkanRenderer {
    pub(crate) dev: VulkanDevice,
    pub(crate) phd: PhysicalDevice,
    pub(crate) queue: RenderQueue,
    pub(crate) command_pool: vk::CommandPool,
    pub(crate) cmd: vk::CommandBuffer,
    pub(crate) pipeline_cache: vk::PipelineCache,
    pub(crate) pipelines: HashMap<vk::Format, CompositePipelines>,
    /// Per-format world anti-aliasing pipelines (windows + iced AA). Built on
    /// demand only when the live `AA_MODE` is non-Off; the plain `pipelines`
    /// above stay the untouched default path. See `pipeline.composite::aa`.
    pub(crate) aa_pipelines: HashMap<vk::Format, AaComposite>,
    /// Reusable per-surface mipped copies for the trilinear/aniso AA modes
    /// (`RefCell` so the per-frame acquire + record borrow cleanly alongside the
    /// other renderer field borrows in `submit`). Empty until such a mode runs.
    pub(crate) mipgen: std::cell::RefCell<super::mipgen::MipGen>,
    /// Whether AA was active last frame — drives lazy build on activation and
    /// resource teardown on deactivation (see `teardown_aa`).
    pub(crate) aa_was_active: bool,
    /// Generic native fullscreen-shader passes, keyed by `(shader id, format)`.
    /// Built on demand from the scene's `DrawOp::ShaderPass` and held for the
    /// renderer's lifetime. The parallax background (SDR + HDR variants) is one
    /// such pass; the kernel keeps no shader-specific knowledge.
    pub(crate) shader_passes: HashMap<(u64, vk::Format), FullscreenPass>,
    /// Per-OUTPUT resources — see [`OutputResources`]. Every monitor gets its own;
    /// `submit_frame` runs once per output pass.
    pub(crate) outputs: HashMap<std::sync::Arc<str>, OutputResources>,
    /// Which output the pass being composed belongs to, pushed by the frame driver
    /// before the pass exactly as the facts are. Empty on the winit /
    /// single-output path, which is one output and so keys consistently.
    pub(crate) output: std::sync::Arc<str>,
    /// This renderer's cursor into the output-retirement mailbox.
    pub(crate) retired_cursor: u64,
    /// The worker's decorated world band for this frame, handed along the draw
    /// seam. TAKEN by the frame that presents it — a band left in place would
    /// freeze the desktop — so it must be re-supplied every frame it is wanted.
    pub(crate) after_band: Option<smithay::backend::allocator::dmabuf::Dmabuf>,
    /// What the world being DRAWN asks of the engine this frame.
    ///
    /// Set by the frame driver before the frame, from that world's own
    /// `PipelineState`. It is a renderer field rather than a `submit_frame`
    /// parameter because `submit_frame` is reached through smithay's
    /// `Frame::finish`, so there is no call site in this crate to thread it
    /// through — and per-RENDERER is the right scope anyway: these gate device
    /// work, and on a multi-GPU host each renderer must answer for its own device
    /// rather than share one process-wide answer.
    pub(crate) facts: compositor_pipeline_abi_worldset_base::base::Facts,
    /// Per-format HDR composite pipelines (M5 1a), created on demand only when
    /// the HDR path is active. The SDR `pipelines` above are untouched.
    pub(crate) hdr_pipelines: HashMap<vk::Format, crate::hdr_composite::HdrComposite>,
    /// HDR output path active (COMPOSITOR_HDR + capable display); set from the
    /// backend. When true `submit_frame` composites via `hdr_pipelines` and
    /// outputs PQ/BT.2020.
    pub(crate) hdr_enabled: bool,
    pub(crate) descriptor_pool: vk::DescriptorPool,
    pub(crate) timeline: vk::Semaphore,
    /// Binary, SYNC_FD-exportable semaphore signaled by each render submit; the
    /// native KMS path exports it directly as a `sync_file` for the atomic-commit
    /// IN_FENCE.
    pub(crate) render_semaphore: vk::Semaphore,
    /// VkFence signaled by each native-path submit — CPU pacing for the reused
    /// command buffer. Created signaled.
    pub(crate) frame_fence: vk::Fence,
    /// The display's DRM device fd, set on the native backend (None under
    /// winit). Its presence selects the native KMS IN_FENCE path.
    pub(crate) drm_fd: Option<smithay::backend::drm::DrmDeviceFd>,
    /// The native KMS IN_FENCE path. DEFAULT IS ON — only
    /// `COMPOSITOR_RENDERER_SYNC=sync` turns it off (synchronous
    /// `device_wait_idle` submit); the name is historical, from when it was the
    /// opt-in.
    pub(crate) native_fence_optin: bool,
    /// `infence_fallback_sync`: run the first-frame fence self-test and degrade
    /// to synchronous mode if the exported fence never signals. Plain `infence`
    /// performs NO validation — the raw path, so a broken fence stack can be
    /// observed behaving as it actually does.
    pub(crate) fence_fallback_optin: bool,
    /// One exported sync_file has been observed to actually signal (first-frame
    /// self-test, `infence_fallback_sync` only). Until then each export is
    /// validated; a dud fence flips `native_fence_optin` off permanently
    /// instead of freezing the commit.
    pub(crate) fence_validated: bool,
    /// Throttle for the per-frame native-fence-export warning (once/min).
    pub(crate) last_fence_warn: Option<std::time::Instant>,
    /// Post-scene capture targets for THIS frame: the registry's entry dmabufs to
    /// copy the composed scene into. Set by the backend before `render_frame`;
    /// consumed (and cleared) in `submit_frame`.
    pub(crate) capture_targets:
        Vec<(Dmabuf, Option<smithay::utils::Rectangle<i32, smithay::utils::Physical>>)>,
    /// Capture-target dmabufs imported as TRANSFER_DST images, re-imported only
    /// when the target set changes (the leak fix lives in `capture.blit`).
    pub(crate) capture_cache: CaptureCache,
    /// Reusable host-visible staging buffer for SHM uploads (grows on demand),
    /// so steady-state SHM updates allocate no new host memory.
    pub(crate) shm_staging: StagingBuffer,
    pub(crate) frame_counter: u64,
    pub(crate) debug_flags: DebugFlags,
    pub(crate) downscale: TextureFilter,
    pub(crate) upscale: TextureFilter,
    pub(crate) context_id: ContextId<VulkanTexture>,
    /// Render-target objects parked by `VulkanFramebuffer::drop` — the GPU may
    /// still be writing to them (native IN_FENCE path). Destroyed by
    /// `drain_retired` at points where the using frame is provably complete.
    pub(crate) retired: std::sync::Arc<std::sync::Mutex<Vec<crate::frame::RetiredTarget>>>,
    /// Arc pins for every texture the frame currently being recorded samples
    /// (pushed by `render_texture_from_to`), moved to `in_flight_textures` at
    /// submit — so a client texture whose last other handle drops mid-flight
    /// (window close) isn't destroyed while the GPU still reads it.
    pub(crate) pinned_textures: Vec<crate::texture::VulkanTexture>,
    /// The previously submitted frame's texture pins; dropped at the drain
    /// point once that frame has provably completed.
    pub(crate) in_flight_textures: Vec<crate::texture::VulkanTexture>,
    /// Live scanout-target imports, keyed by dmabuf identity. Entries whose
    /// dmabuf has died (an output resize/mode change retires its whole
    /// swapchain) are reaped at the drain point in `submit_frame`. Membership is
    /// also the record of which buffers hold one of our composites, which is
    /// what `bind` reads to decide whether the target can be acquired
    /// content-preserving or must be discarded and fully cleared.
    pub(crate) target_cache: HashMap<smithay::backend::allocator::dmabuf::WeakDmabuf, CachedTarget>,
    /// Sampled dmabuf imports kept across frames, keyed by source dmabuf
    /// identity. The `Arc` inside `VulkanTexture` owns the image/memory/view, so
    /// an entry dropped here is destroyed only once every other handle
    /// (including this frame's `pinned_textures`) is gone.
    pub(crate) import_cache:
        HashMap<smithay::backend::allocator::dmabuf::WeakDmabuf, VulkanTexture>,
    /// Textures served from `import_cache` since the last submit. Their producer
    /// (iced's or bevy's own wgpu device, a client) has written to the underlying
    /// dmabuf since we last sampled it, so the frame that samples them opens with
    /// one batched foreign-queue acquire — the in-command-buffer replacement for
    /// the per-import `transition_to_sampled` the cache skips, minus only its
    /// submit+fence.
    ///
    /// Holds `VulkanTexture` PINS, not bare `vk::Image`, and is deduped on push.
    /// Scene building imports even on frames that never submit (smithay's damage
    /// tracker early-returns before `render()` when nothing changed — the common
    /// idle case), so this list both outlives individual frames and must not grow
    /// unbounded: the pin keeps a since-evicted image valid, and the dedup bounds
    /// the length by the number of distinct live surfaces.
    pub(crate) pending_acquires: Vec<VulkanTexture>,
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
    /// Which output the pass being composed belongs to. Pushed by the frame
    /// driver before the pass, the way the facts are — `submit_frame` is reached
    /// through smithay's `Frame::finish` and has no way to learn it otherwise.
    pub fn set_render_output(&mut self, output: &std::sync::Arc<str>) {
        self.output = std::sync::Arc::clone(output);
    }

    /// Release what every REMOVED output owned.
    ///
    /// Drains the kernel's own output-retirement mailbox with this renderer's own
    /// cursor — the background worker reads the same mailbox for the same reason,
    /// and the two are independent readers. Without this an unplugged 4K
    /// display's intermediate targets, `content` and `history` stay resident for
    /// the session.
    pub fn reclaim_removed_outputs(&mut self) {
        if compositor_kernel_graphic_bridge_publish_retire::retire::retired_epoch()
            == self.retired_cursor
        {
            return;
        }
        let (gone, next) = compositor_kernel_graphic_bridge_publish_retire::retire::retired_since(
            self.retired_cursor,
        );
        self.retired_cursor = next;
        for output in gone {
            if let Some(r) = self.outputs.remove(output.as_str()) {
                info!("renderer: output {output:?} removed — released its intermediate targets");
                r.destroy(&self.dev);
            }
        }
    }

    /// What THIS output exported for the worker this frame.
    pub fn shared_bands(&self) -> (Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>, Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>) {
        match self.outputs.get(&self.output) {
            Some(r) => (r.content_shared.clone(), r.windows_shared.clone()),
            None => (None, None),
        }
    }

    /// The warp grid this device last produced. Cloned, not taken: it stays valid
    /// until the producer changes, and a frame that records no new grid must not
    /// blank the pointer correction.
    pub fn warp_grid(&self) -> Option<std::sync::Arc<Vec<[f32; 2]>>> {
        self.outputs.get(&self.output)?.warp_grid.clone()
    }

    /// Take what the last frame collected, for the world that was drawn.
    pub fn take_world_set(&mut self) -> Option<std::sync::Arc<compositor_pipeline_abi_worldset_base::base::WorldSet>> {
        self.outputs.get_mut(&self.output)?.world_set.take()
    }

    /// The facts of the world being drawn, set by the frame driver each frame.
    pub fn set_pipeline_facts(
        &mut self,
        facts: compositor_pipeline_abi_worldset_base::base::Facts,
    ) {
        self.facts = facts;
    }

    /// Whether the drawn world's bundle wants window textures, and therefore
    /// whether a SHM upload must be allocated exportable.
    ///
    /// LIVE, not settled at startup: the selected bundle changes whenever the user
    /// picks a shader. A change is not stranded — `shm_cache` refuses to reuse a
    /// cached texture whose shareability no longer matches, so each surface
    /// reallocates correctly on its next commit, in both directions.
    pub(crate) fn wants_shared_shm(&self) -> bool {
        compositor_pipeline_abi_seam_base::base::Requires::from_bits(self.facts.requires)
            .textures()
    }

    pub(crate) fn context_id_value(&self) -> ContextId<VulkanTexture> {
        self.context_id.clone()
    }

    /// Destroy retired render-target objects. Call ONLY where the frames that
    /// used them are provably complete: after the pre-record `frame_fence`
    /// wait (native path), after a `device_wait_idle`, or at renderer drop.
    pub(crate) fn drain_retired(&self) {
        let mut retired = match self.retired.lock() {
            Ok(list) => list,
            Err(poisoned) => poisoned.into_inner(),
        };
        for target in retired.drain(..) {
            unsafe {
                if target.view != vk::ImageView::null() {
                    self.dev.device.destroy_image_view(target.view, None);
                }
                if target.image != vk::Image::null() {
                    self.dev.device.destroy_image(target.image, None);
                }
                if target.memory != vk::DeviceMemory::null() {
                    self.dev.device.free_memory(target.memory, None);
                }
            }
        }
    }

    /// Destroy one cached scanout-target import. Callers must be at a point
    /// where the frame that last used it has provably completed.
    pub(crate) fn destroy_target(dev: &VulkanDevice, t: &CachedTarget) {
        unsafe {
            dev.device.destroy_image_view(t.view, None);
            dev.device.destroy_image(t.image, None);
            dev.device.free_memory(t.memory, None);
        }
    }

    /// Drop cache entries whose source dmabuf has been freed — an output resize
    /// or mode change retires its whole swapchain; a surface resize or release
    /// mints a new backing buffer. Same safety precondition as
    /// [`drain_retired`], and called from the same place.
    ///
    /// Dropping an `import_cache` entry only releases OUR handle: a texture this
    /// frame still samples stays alive through its `pinned_textures` pin, so
    /// this cannot free something the GPU is reading. `target_cache` entries own
    /// their objects outright, hence the explicit destroy.
    pub(crate) fn reap_targets(&mut self) {
        self.import_cache.retain(|w, _| w.upgrade().is_some());
        let dead: Vec<_> = self
            .target_cache
            .keys()
            .filter(|w| w.upgrade().is_none())
            .cloned()
            .collect();
        for key in dead {
            if let Some(t) = self.target_cache.remove(&key) {
                Self::destroy_target(&self.dev, &t);
            }
        }
    }

    /// Provide the display's DRM device fd (native backend). Its presence is
    /// what switches `finish()` to the KMS IN_FENCE path — unless
    /// `COMPOSITOR_RENDERER_SYNC=sync` opted out, in which case the synchronous
    /// submit is kept.
    pub fn set_drm_fd(&mut self, fd: smithay::backend::drm::DrmDeviceFd) {
        self.drm_fd = Some(fd);
        if self.native_fence_optin {
            info!("sync mode: native KMS IN_FENCE (sync_file export, no per-frame device_wait_idle)");
            stats::set_sync_mode("native KMS IN_FENCE (sync_file)");
        } else {
            info!("sync mode: synchronous device_wait_idle (renderer_sync == \"sync\")");
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

    /// True when the native KMS IN_FENCE path should be used: not opted out of
    /// (`COMPOSITOR_RENDERER_SYNC=sync`) and a DRM fd is present (native).
    pub(crate) fn use_native_fence(&self) -> bool {
        self.native_fence_optin && self.drm_fd.is_some()
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.dev.device.device_wait_idle();
            self.drain_retired();
            // Scanout-target imports: the framebuffers no longer own these.
            for (_, t) in self.target_cache.drain() {
                Self::destroy_target(&self.dev, &t);
            }
            // Drop texture pins BEFORE destroy_device below — `TextureInner`
            // destroys through its own device clone on last-handle drop.
            self.in_flight_textures.clear();
            self.pinned_textures.clear();
            self.import_cache.clear();
            // Capture-target imports + the reusable SHM staging buffer.
            self.capture_cache.destroy(&self.dev);
            self.shm_staging.destroy(&self.dev);
            let passes: Vec<_> = self.shader_passes.drain().collect();
            for (_, p) in passes {
                p.destroy(&self.dev);
            }
            let outputs: Vec<_> = self.outputs.drain().collect();
            for (_, r) in outputs {
                r.destroy(&self.dev);
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
