//! Command recording for one composition frame: begin -> layout transitions
//! (synchronization2 barriers) -> composition pass -> transition for scanout
//! export -> end. Phase 4 Step 3 — real.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("vulkan call failed: {0}")]
    Vk(String),
}

/// Record one composition frame. `compose` receives the live command buffer
/// between the begin/end of the rendering pass (this is where
/// `vulkan.element` draws land).
pub fn record_composition(
    device: &VulkanDevice,
    cmd: vk::CommandBuffer,
    target_image: vk::Image,
    target_view: vk::ImageView,
    extent: (u32, u32),
    clear: [f32; 4],
    pipelines: &compositor_kernel_vulkan_pipeline_composite_base::composite::CompositePipelines,
    // Runs after the command buffer begins but BEFORE the composite render pass
    // — for pre-pass work that must be outside the pass (e.g. world anti-aliasing trilinear
    // mip generation: render-to-mip0 + blit-down). No-op for most frames.
    pre: impl FnOnce(vk::CommandBuffer),
    compose: impl FnOnce(vk::CommandBuffer),
) -> Result<(), RecordError> {
    let dev = &device.device;
    let begin_info = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe {
        dev.begin_command_buffer(cmd, &begin_info)
            .map_err(|e| RecordError::Vk(format!("begin: {e}")))?;
    }

    pre(cmd);

    let subresource = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    // UNDEFINED -> COLOR_ATTACHMENT_OPTIMAL.
    let to_attachment = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
        .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .image(target_image)
        .subresource_range(subresource);
    let dep = vk::DependencyInfo::default()
        .image_memory_barriers(std::slice::from_ref(&to_attachment));
    unsafe { dev.cmd_pipeline_barrier2(cmd, &dep) };

    compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
        device, cmd, target_view, extent, clear,
    );
    compose(cmd);
    compositor_kernel_vulkan_pipeline_composite_base::composite::end(device, cmd);
    let _ = pipelines;

    // COLOR_ATTACHMENT_OPTIMAL -> GENERAL, and RELEASE to the display engine
    // (VK_QUEUE_FAMILY_FOREIGN_EXT) so the driver flushes/decompresses tiled/DCC
    // framebuffers before KMS scans them out (else AMD white-screens). Mirror of
    // the import-side acquire; MULTIPLANE_SUPPORT is what enables the extension.
    let mut to_external = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
        .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .image(target_image)
        .subresource_range(subresource);
    if compositor_kernel_vulkan_device_factory_base::factory::MULTIPLANE_SUPPORT {
        to_external = to_external
            .src_queue_family_index(device.queue_family_index)
            .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT);
    }
    let dep = vk::DependencyInfo::default()
        .image_memory_barriers(std::slice::from_ref(&to_external));
    unsafe { dev.cmd_pipeline_barrier2(cmd, &dep) };

    unsafe {
        dev.end_command_buffer(cmd)
            .map_err(|e| RecordError::Vk(format!("end: {e}")))?;
    }
    Ok(())
}
