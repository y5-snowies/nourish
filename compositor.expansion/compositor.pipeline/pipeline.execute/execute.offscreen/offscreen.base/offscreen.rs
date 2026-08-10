//! Phase-0 offscreen compositing (opt-in). Instead of rendering the scene
//! straight into the scanout dmabuf, render it into an owned `content` image,
//! then a passthrough pass samples `content` into the swapchain. On its own this
//! is a pixel-identical no-op; it exists so later passes can run AFTER window
//! compositing and SAMPLE the composited scene (vignette-above, glass backdrop —
//! see `document/SHADER_PIPELINE.md`). Built only when a loaded bundle REQUIRES one
//! of the images it produces (`composited_scene`/`window_layer`/`previous_frame`),
//! so with no such bundle the direct composite path stays byte-for-byte untouched.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_pipeline_execute_effect_base::effect::EffectPass;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;

/// The passthrough shader, naga-compiled to SPIR-V by `build.rs`.
pub const PASSTHROUGH_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/passthrough.spv"));

fn device_local(mem: &vk::PhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    (0..mem.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && mem.memory_types[i as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    })
}

fn subrange() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn barrier(
    dev: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags2,
    dst_stage: vk::PipelineStageFlags2,
    src_access: vk::AccessFlags2,
    dst_access: vk::AccessFlags2,
) {
    let b = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src_stage)
        .dst_stage_mask(dst_stage)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .old_layout(old)
        .new_layout(new)
        .image(image)
        .subresource_range(subrange());
    unsafe {
        dev.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&b)),
        );
    }
}

/// An owned offscreen color image the scene is composited into.
struct Target {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    w: u32,
    h: u32,
    format: vk::Format,
    /// Allocation size, needed by a second device importing this memory.
    size: u64,
}

impl Target {
    fn create(
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        format: vk::Format,
        w: u32,
        h: u32,
        usage: vk::ImageUsageFlags,
        shareable: bool,
    ) -> Result<Self, VulkanError> {
        let device = &dev.device;
        // `shareable` adds OPAQUE_FD external memory so a second logical device on
        // the SAME physical device can bind the same allocation — how `content`
        // reaches the worker (stage 4). Costs an allocation flag; tiling is
        // unchanged, unlike a dmabuf export.
        let mut ext = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let mut info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width: w, height: h, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        if shareable {
            info = info.push_next(&mut ext);
        }
        let image = unsafe {
            device
                .create_image(&info, None)
                .map_err(|e| VulkanError::Vk(format!("content image: {e}")))?
        };
        let req = unsafe { device.get_image_memory_requirements(image) };
        let mem_idx = device_local(mem, req.memory_type_bits)
            .ok_or_else(|| VulkanError::Vk("content: no device-local memory".into()))?;
        let mut ext_alloc = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let mut alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(mem_idx);
        if shareable {
            alloc = alloc.push_next(&mut ext_alloc);
        }
        let memory = unsafe {
            device
                .allocate_memory(&alloc, None)
                .map_err(|e| VulkanError::Vk(format!("content memory: {e}")))?
        };
        unsafe {
            device
                .bind_image_memory(image, memory, 0)
                .map_err(|e| VulkanError::Vk(format!("content bind: {e}")))?
        };
        let view = unsafe {
            device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(subrange()),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("content view: {e}")))?
        };
        Ok(Self { image, memory, view, w, h, format, size: req.size })
    }

    fn destroy(&self, dev: &VulkanDevice) {
        unsafe {
            dev.device.destroy_image_view(self.view, None);
            dev.device.destroy_image(self.image, None);
            dev.device.free_memory(self.memory, None);
        }
    }
}

/// The passthrough pass + its reused `content` target.
pub struct ContentPath {
    pass: EffectPass,
    target: Option<Target>,
    /// Persistent previous-frame image (`history`), kept + filled only when a
    /// loaded pipeline references the built-in `history` target. `content` is
    /// copied into it at the end of each frame, so next frame samples it.
    history: Option<Target>,
    /// The composited WINDOW LAYER (`windows`): client windows over transparency,
    /// background excluded. Consumption-gated exactly like `history` — allocated
    /// only when a loaded pipeline samples it, so it costs nothing otherwise.
    ///
    /// This is what lets a pass reach window CONTENT without touching client
    /// buffers: by the time windows are drawn here, dmabuf and SHM surfaces have
    /// both been resolved into ordinary images, so a shader reads them the same way
    /// and nothing has to cross a device boundary.
    windows: Option<Target>,
    /// `content` shared for a second device, minted with the target and dropped
    /// with it. `Arc` so readers can hold it without owning the fd.
    content_share: Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    /// The window layer shared for a second device, on the same terms as
    /// `content_share`. Minted with the layer, so it exists only for the bundles
    /// that sample `windows` at all.
    windows_share: Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    format: vk::Format,
}

impl ContentPath {

    /// Build the passthrough pipeline for `format` (rebuilt by the caller if the
    /// output format ever changes — see [`Self::format`]).
    pub fn new(
        device: &VulkanDevice,
        cache: vk::PipelineCache,
        format: vk::Format,
    ) -> Result<Self, VulkanError> {
        let pass = EffectPass::create(
            device, cache, format, 1, PASSTHROUGH_SPV, None, "vs_main", "fs_main", 16, None, None,
        )?;
        Ok(Self { pass, target: None, history: None, windows: None, content_share: None, windows_share: None, format })
    }

    /// A view of the persistent `history` image (previous frame's `content`), or
    /// null when no pipeline references it. Bound as the `HISTORY` input.
    pub fn history_view(&self) -> vk::ImageView {
        self.history.as_ref().map(|t| t.view).unwrap_or(vk::ImageView::null())
    }

    /// This frame's `content` shared as an `OPAQUE_FD`, or `None` before the first
    /// composite. Exported ONCE per allocation, not per frame.
    ///
    /// Stage 4 hands this to the worker so an after-content pass can run there:
    /// one sample of a band the compositor already composited, keeping the
    /// engine's AA, HDR, damage and occlusion, instead of the shader recompositing
    /// every window itself. See `SHADER_PIPELINE_WORKER.md` stage 4.
    pub fn content_share(
        &self,
    ) -> Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>> {
        self.content_share.clone()
    }

    /// A view of the composited window layer, or null when no pipeline samples it.
    /// The window layer's export, for a worker running an after pass that samples
    /// `windows`. `None` until a bundle asks for the layer at all.
    pub fn windows_share(
        &self,
    ) -> Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>> {
        self.windows_share.clone()
    }

    pub fn windows_view(&self) -> vk::ImageView {
        self.windows.as_ref().map(|t| t.view).unwrap_or(vk::ImageView::null())
    }

    /// The color format the passthrough pipeline was built for.
    pub fn format(&self) -> vk::Format {
        self.format
    }

    fn ensure_target(
        &mut self,
        device: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        extent: (u32, u32),
        format: vk::Format,
    ) -> Result<(), VulkanError> {
        let (w, h) = (extent.0.max(1), extent.1.max(1));
        let stale = match &self.target {
            Some(t) => t.w != w || t.h != h || t.format != format,
            None => true,
        };
        if stale {
            if let Some(t) = self.target.take() {
                t.destroy(device);
            }
            // TRANSFER_SRC so the frame's `content` can be copied into `history`.
            self.target = Some(Target::create(
                device,
                mem,
                format,
                w,
                h,
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_SRC,
                true,
            )?);
            // Export alongside the allocation: one fd per content image, replaced
            // only when the image is (resize / format change).
            self.content_share = self.target.as_ref().and_then(|t| {
                compositor_kernel_vulkan_memory_external_base::external::export(
                    device, t.memory, t.size, t.format, t.w, t.h,
                )
                .ok()
                .map(std::sync::Arc::new)
            });
        }
        Ok(())
    }

    /// Ensure the persistent `history` image matches the current extent/format.
    /// Returns `true` when it was (re)created this call, so the caller clears it
    /// to black before the first sample (undefined contents otherwise).
    fn ensure_history(
        &mut self,
        device: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        extent: (u32, u32),
        format: vk::Format,
    ) -> Result<bool, VulkanError> {
        let (w, h) = (extent.0.max(1), extent.1.max(1));
        let stale = match &self.history {
            Some(t) => t.w != w || t.h != h || t.format != format,
            None => true,
        };
        if stale {
            if let Some(t) = self.history.take() {
                t.destroy(device);
            }
            // SAMPLED (read by history passes) + TRANSFER_DST (copy target).
            self.history = Some(Target::create(
                device,
                mem,
                format,
                w,
                h,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                false,
            )?);
        }
        Ok(stale)
    }

    /// Ensure the window-layer image matches the current extent/format. Modelled
    /// on [`Self::ensure_history`]; COLOR_ATTACHMENT (windows are drawn into it)
    /// plus SAMPLED (an after-content pass reads it).
    fn ensure_windows(
        &mut self,
        device: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        extent: (u32, u32),
        format: vk::Format,
    ) -> Result<(), VulkanError> {
        let (w, h) = (extent.0.max(1), extent.1.max(1));
        let stale = match &self.windows {
            Some(t) => t.w != w || t.h != h || t.format != format,
            None => true,
        };
        if stale {
            if let Some(t) = self.windows.take() {
                t.destroy(device);
            }
            self.windows = Some(Target::create(
                device,
                mem,
                format,
                w,
                h,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                true,
            )?);
            self.windows_share = self.windows.as_ref().and_then(|t| {
                compositor_kernel_vulkan_memory_external_base::external::export(
                    device, t.memory, t.size, t.format, t.w, t.h,
                )
                .ok()
                .map(std::sync::Arc::new)
            });
        }
        Ok(())
    }

    /// Record the whole frame: `pre` (outside any pass), the scene `compose` into
    /// `content`, then the post-content stage into the swapchain. `post_pre` runs
    /// after `content` is sampleable and before the swapchain pass opens (for
    /// after-content intermediate passes); `post_draw` draws the final image
    /// inside the swapchain pass and returns whether it drew — when it returns
    /// `false` (no after-content pipeline), the built-in passthrough runs instead.
    /// Both receive `(cmd, content_view, history_view)`; `history_view` is null
    /// unless `keep_history`. When `keep_history`, this frame's `content` is copied
    /// into the persistent `history` image at the end, so next frame samples it.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        cmd: vk::CommandBuffer,
        swap_image: vk::Image,
        swap_view: vk::ImageView,
        extent: (u32, u32),
        format: vk::Format,
        clear: [f32; 4],
        keep_history: bool,
        keep_windows: bool,
        // Stage 4: the worker's DECORATED world band. When present the swapchain
        // shows THIS rather than the locally-run after pass — the effect already
        // happened, off-thread, over the `content` this path published last frame.
        band_view: Option<vk::ImageView>,
        pre: impl FnOnce(vk::CommandBuffer),
        compose: impl FnOnce(vk::CommandBuffer),
        compose_windows: impl FnOnce(vk::CommandBuffer),
        compose_screen: impl FnOnce(vk::CommandBuffer),
        post_pre: impl FnOnce(vk::CommandBuffer, vk::ImageView, vk::ImageView, vk::ImageView),
        post_draw: impl FnOnce(vk::CommandBuffer, vk::ImageView, vk::ImageView, vk::ImageView) -> bool,
    ) -> Result<(), VulkanError> {
        self.ensure_target(device, mem, extent, format)?;
        // Both layers are CONSUMPTION-GATED, and the release half is as
        // load-bearing as the allocate half.
        //
        // `windows` used to be allocated when a bundle asked for it and never let
        // go. The draw below is gated on the IMAGE existing, not on the
        // requirement, while the caller's descriptor-set pass skips windows a
        // bundle owns unless the layer is wanted — so once any bundle had asked
        // for the layer, every later bundle that owned the band but did not want
        // it composed the window pass against sets that were never allocated, and
        // the frame died on `textured op has a set`. Releasing here keeps the
        // layer's existence and the requirement the same fact.
        //
        // It is also a fullscreen image per output: holding one for a bundle that
        // never samples it is the cost this gating exists to avoid.
        let fresh_history =
            if keep_history { self.ensure_history(device, mem, extent, format)? } else { false };
        if !keep_history {
            if let Some(t) = self.history.take() {
                t.destroy(device);
            }
        }
        if keep_windows {
            self.ensure_windows(device, mem, extent, format)?;
        } else if let Some(t) = self.windows.take() {
            t.destroy(device);
            self.windows_share = None;
        }
        self.pass.begin_frame(device);
        let content = self.target.as_ref().expect("target ensured");
        let history = self.history.as_ref();
        let history_view = history.map(|t| t.view).unwrap_or(vk::ImageView::null());
        let windows = self.windows.as_ref();
        let windows_view = windows.map(|t| t.view).unwrap_or(vk::ImageView::null());
        let dev = &device.device;

        unsafe {
            dev.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(|e| VulkanError::Vk(format!("offscreen begin: {e}")))?;
        }

        // A freshly (re)created history image is UNDEFINED — clear it to black and
        // leave it SHADER_READ so this frame's history passes sample defined pixels
        // (the previous-frame contents arrive via the end-of-frame copy below).
        if fresh_history {
            if let Some(h) = history {
                barrier(
                    dev, cmd, h.image,
                    vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::TRANSFER,
                    vk::AccessFlags2::empty(), vk::AccessFlags2::TRANSFER_WRITE,
                );
                let black = vk::ClearColorValue { float32: [0.0, 0.0, 0.0, 0.0] };
                unsafe {
                    dev.cmd_clear_color_image(
                        cmd, h.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &black,
                        std::slice::from_ref(&subrange()),
                    );
                }
                barrier(
                    dev, cmd, h.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::TRANSFER, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::TRANSFER_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
                );
            }
        }

        pre(cmd);

        // Window layer → its own image, BEFORE the content pass. Additive: the
        // existing compose order is untouched, and this costs nothing unless a
        // pipeline samples `windows`. Cleared to TRANSPARENT so a shader can tell
        // window pixels from empty desktop by alpha.
        if let Some(w) = windows {
            barrier(
                dev, cmd, w.image,
                vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            );
            compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
                device, cmd, w.view, extent, [0.0; 4], vk::AttachmentLoadOp::CLEAR,
            );
            compose_windows(cmd);
            compositor_kernel_vulkan_pipeline_composite_base::composite::end(device, cmd);
            barrier(
                dev, cmd, w.image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
            );
        }

        // Scene → content.
        barrier(
            dev, cmd, content.image,
            vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        // CLEAR, never LOAD: the barrier above acquires `content` from UNDEFINED
        // (contents discarded), and the offscreen path always redraws in full —
        // the after-content passes sample neighbouring pixels, so it cannot be
        // damage-scissored. Nothing to preserve.
        compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
            device, cmd, content.view, extent, clear, vk::AttachmentLoadOp::CLEAR,
        );
        compose(cmd);
        compositor_kernel_vulkan_pipeline_composite_base::composite::end(device, cmd);

        // content → sampled.
        barrier(
            dev, cmd, content.image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
        );
        let content_view = content.view;

        // After-content intermediate passes (sample `content`/`history`), outside
        // the pass — but NOT when the worker already ran them: doing both would pay
        // for the effect twice and then discard the local copy.
        if band_view.is_none() {
            post_pre(cmd, content_view, history_view, windows_view);
        }

        // swapchain → color attachment.
        barrier(
            dev, cmd, swap_image,
            vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );

        // Final image → swapchain: the after-content output pass, or (if none) the
        // built-in passthrough content → swapchain.
        // CLEAR for the same reason: the swapchain is acquired from UNDEFINED
        // here, and the output pass (or the passthrough below) covers the full
        // extent every frame.
        compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
            device, cmd, swap_view, extent, clear, vk::AttachmentLoadOp::CLEAR,
        );
        // Three ways the swapchain gets filled, in order of precedence:
        //   1. the worker's decorated band (stage 4) — the after pass ran there;
        //   2. an after-content pass running HERE;
        //   3. the built-in passthrough of `content`.
        // The same passthrough pipeline serves 1 and 3; only the source differs,
        // so presenting the worker's band costs no extra pipeline.
        let passthrough_src = match band_view {
            Some(v) => Some(v),
            None => (!post_draw(cmd, content_view, history_view, windows_view))
                .then_some(content_view),
        };
        if let Some(src) = passthrough_src {
            let set = self.pass.input_set(device, &[src])?;
            let push: Vec<u8> = [extent.0 as f32, extent.1 as f32, 0.0, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect();
            self.pass.draw(device, cmd, Some(set), None, None, &push);
        }
        // SCREEN band (UI + pointer) on top of the post-processed world content —
        // it must not be affected by the after-content passes.
        compose_screen(cmd);
        compositor_kernel_vulkan_pipeline_composite_base::composite::end(device, cmd);

        // Copy this frame's `content` (world band: background + windows) into the
        // persistent `history` image for next frame — after every history/content
        // read above. `history` is thus the previous frame's WORLD content (not the
        // final swapchain: it excludes after-content passes + the SCREEN UI, which
        // is what glass/motion-blur backdrops actually want).
        if keep_history {
            if let Some(h) = history {
                barrier(
                    dev, cmd, content.image,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER, vk::PipelineStageFlags2::TRANSFER,
                    vk::AccessFlags2::SHADER_SAMPLED_READ, vk::AccessFlags2::TRANSFER_READ,
                );
                barrier(
                    dev, cmd, h.image,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER, vk::PipelineStageFlags2::TRANSFER,
                    vk::AccessFlags2::SHADER_SAMPLED_READ, vk::AccessFlags2::TRANSFER_WRITE,
                );
                let layers = vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                };
                let region = vk::ImageCopy {
                    src_subresource: layers,
                    src_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                    dst_subresource: layers,
                    dst_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                    extent: vk::Extent3D { width: content.w, height: content.h, depth: 1 },
                };
                unsafe {
                    dev.cmd_copy_image(
                        cmd,
                        content.image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        h.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        std::slice::from_ref(&region),
                    );
                }
                // history → SHADER_READ for next frame's history passes.
                barrier(
                    dev, cmd, h.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::TRANSFER, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::TRANSFER_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
                );
            }
        }

        // swapchain → GENERAL for the scanout consumer.
        barrier(
            dev, cmd, swap_image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::GENERAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::empty(),
        );

        unsafe {
            dev.end_command_buffer(cmd)
                .map_err(|e| VulkanError::Vk(format!("offscreen end: {e}")))?;
        }
        Ok(())
    }

    pub fn destroy(&self, device: &VulkanDevice) {
        if let Some(t) = &self.target {
            t.destroy(device);
        }
        if let Some(t) = &self.history {
            t.destroy(device);
        }
        if let Some(t) = &self.windows {
            t.destroy(device);
        }
        self.pass.destroy(device);
    }
}
