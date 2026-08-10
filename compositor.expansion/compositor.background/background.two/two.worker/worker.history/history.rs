//! `history` on the worker: the previous PASS's `content`, kept on this device.
//!
//! Derived, not transported — the opposite choice from `windows`, and for a
//! reason. `history` is not a distinct thing the compositor draws; it is a copy
//! of `content` taken at frame end. The worker already receives `content` (stage
//! 4's transport), so it can take the same copy on its own side and owe the
//! compositor nothing: no second export, no extra fd, no coupling to when the
//! compositor rotates its own history. `windows` cannot be derived that way,
//! which is why that one is exported instead.
//!
//! # The semantic difference, which is real and visible
//!
//! The compositor's `history` is the previous FRAME. This is the previous PASS —
//! and the worker runs at its own rate, below the compositor's whenever the
//! rate/cap knobs say so. A trail built from it is therefore longer and coarser
//! than the same bundle produces inline: fewer samples, further apart. That is
//! not a defect to correct, it is what offloading a temporal effect means, and a
//! bundle whose look depends on frame-exact decay should stay inline.
//!
//! # Why a draw and not a `cmd_copy_image`
//!
//! A copy would need `content` in `TRANSFER_SRC_OPTIMAL`. `content` is the
//! compositor's image, reached here through an `OPAQUE_FD` import, and the whole
//! reason the existing stage-4 path is safe is that the worker only ever SAMPLES
//! it — it never transitions an image the other device owns. Sampling it into a
//! local attachment keeps that property: the shared image is used exactly as the
//! graph already uses it, and the only layout this crate moves is its own.

use ash::vk;
use compositor_background_two_worker_device::device::Device;
use compositor_pipeline_execute_effect_base::effect::EffectPass;

/// One pane's persistent previous-pass image, plus the passthrough pipeline that
/// fills it.
pub struct History {
    pub image: vk::Image,
    pub view: vk::ImageView,
    memory: vk::DeviceMemory,
    pass: EffectPass,
    extent: (u32, u32),
    format: vk::Format,
    /// Cleared before its first sample. Fresh device memory is undefined, and
    /// undefined here reads as a full-screen flash of whatever the allocator
    /// last had in that page.
    fresh: bool,
}

/// (Re)build `slot` when it does not match `extent`/`format`.
pub fn ensure(
    slot: &mut Option<History>,
    d: &Device,
    extent: (u32, u32),
    format: vk::Format,
) -> Result<(), String> {
    let (w, h) = (extent.0.max(1), extent.1.max(1));
    if slot.as_ref().is_some_and(|x| x.extent == (w, h) && x.format == format) {
        return Ok(());
    }
    if let Some(old) = slot.take() {
        old.destroy(d);
    }
    let dev = &d.dev.device;
    let image = unsafe {
        dev.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(
                    vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::TRANSFER_DST,
                )
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )
        .map_err(|e| format!("worker history image: {e}"))?
    };
    let req = unsafe { dev.get_image_memory_requirements(image) };
    let props =
        unsafe { d.dev.instance.get_physical_device_memory_properties(d.phd.handle()) };
    let idx = (0..props.memory_type_count)
        .find(|&i| {
            req.memory_type_bits & (1 << i) != 0
                && props.memory_types[i as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .ok_or("worker history: no device-local memory")?;
    let memory = unsafe {
        dev.allocate_memory(
            &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(idx),
            None,
        )
        .map_err(|e| format!("worker history memory: {e}"))?
    };
    unsafe {
        dev.bind_image_memory(image, memory, 0)
            .map_err(|e| format!("worker history bind: {e}"))?;
    }
    let view = unsafe {
        dev.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .level_count(1)
                        .layer_count(1),
                ),
            None,
        )
        .map_err(|e| format!("worker history view: {e}"))?
    };
    // The same passthrough the compositor's own offscreen path uses, so "copy
    // content into history" means one thing in this tree, not two.
    let pass = EffectPass::create(
        &d.dev,
        vk::PipelineCache::null(),
        format,
        1,
        compositor_pipeline_execute_offscreen_base::offscreen::PASSTHROUGH_SPV,
        None,
        "vs_main",
        "fs_main",
        16,
        None,
        None,
    )
    .map_err(|e| format!("worker history pass: {e:?}"))?;
    *slot = Some(History { image, view, memory, pass, extent: (w, h), format, fresh: true });
    Ok(())
}

impl History {
    /// Black out a just-allocated image, before anything samples it. No-op once
    /// the first capture has run.
    pub fn prime(&mut self, d: &Device, cmd: vk::CommandBuffer) {
        if !self.fresh {
            return;
        }
        self.fresh = false;
        let dev = &d.dev.device;
        transition(
            dev, cmd, self.image,
            vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::AccessFlags2::empty(), vk::AccessFlags2::TRANSFER_WRITE,
            vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::TRANSFER,
        );
        unsafe {
            dev.cmd_clear_color_image(
                cmd,
                self.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &vk::ClearColorValue { float32: [0.0; 4] },
                &[vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1)],
            );
        }
        transition(
            dev, cmd, self.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags2::TRANSFER_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
            vk::PipelineStageFlags2::TRANSFER, vk::PipelineStageFlags2::FRAGMENT_SHADER,
        );
    }

    /// Sample `content` into this image, so the NEXT pass reads this pass's band.
    /// Call after every pass that sampled `history` has been recorded.
    pub fn capture(&self, d: &Device, cmd: vk::CommandBuffer, content: vk::ImageView) {
        let dev = &d.dev.device;
        self.pass.begin_frame(&d.dev);
        let Ok(set) = self.pass.input_set(&d.dev, &[content]) else { return };
        transition(
            dev, cmd, self.image,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::AccessFlags2::SHADER_SAMPLED_READ, vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        );
        compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
            &d.dev, cmd, self.view, self.extent, [0.0; 4], vk::AttachmentLoadOp::CLEAR,
        );
        let push: Vec<u8> = [self.extent.0 as f32, self.extent.1 as f32, 0.0, 0.0]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        self.pass.draw(&d.dev, cmd, Some(set), None, None, &push);
        compositor_kernel_vulkan_pipeline_composite_base::composite::end(&d.dev, cmd);
        transition(
            dev, cmd, self.image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
        );
    }

    pub fn destroy(self, d: &Device) {
        self.pass.destroy(&d.dev);
        unsafe {
            d.dev.device.destroy_image_view(self.view, None);
            d.dev.device.destroy_image(self.image, None);
            d.dev.device.free_memory(self.memory, None);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn transition(
    dev: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    from: vk::ImageLayout,
    to: vk::ImageLayout,
    src_access: vk::AccessFlags2,
    dst_access: vk::AccessFlags2,
    src_stage: vk::PipelineStageFlags2,
    dst_stage: vk::PipelineStageFlags2,
) {
    let b = vk::ImageMemoryBarrier2::default()
        .image(image)
        .old_layout(from)
        .new_layout(to)
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .src_stage_mask(src_stage)
        .dst_stage_mask(dst_stage)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
        );
    unsafe {
        dev.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&b)),
        );
    }
}
