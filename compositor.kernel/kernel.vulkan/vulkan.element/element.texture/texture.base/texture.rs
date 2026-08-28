//! Textured-surface element drawing: pixel-space src/dst rects -> PushQuad ->
//! composite textured draw. Phase 4 Step 3 — real (descriptor-set update for
//! an imported client image included). The smithay Renderer-trait shim
//! (Path (a)) composes these per element once integration lands.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_pipeline_composite_base::composite::{
    self, CompositePipelines, PushQuad,
};

/// Pixel-space geometry -> NDC/UV push constants.
pub fn quad(
    output_size: (u32, u32),
    dst: (i32, i32, i32, i32),
    src_uv: (f32, f32, f32, f32),
    alpha: f32,
) -> PushQuad {
    let (ow, oh) = (output_size.0 as f32, output_size.1 as f32);
    PushQuad {
        dst: [
            (dst.0 as f32 / ow) * 2.0 - 1.0,
            (dst.1 as f32 / oh) * 2.0 - 1.0,
            (dst.2 as f32 / ow) * 2.0,
            (dst.3 as f32 / oh) * 2.0,
        ],
        src: [src_uv.0, src_uv.1, src_uv.2, src_uv.3],
        color: [1.0, 1.0, 1.0, alpha],
    }
}

/// Point a combined-image-sampler descriptor at an imported client image.
pub fn bind_texture(
    device: &VulkanDevice,
    pipelines: &CompositePipelines,
    descriptor_set: vk::DescriptorSet,
    view: vk::ImageView,
) {
    let image_info = vk::DescriptorImageInfo::default()
        .sampler(pipelines.sampler)
        .image_view(view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
    let write = vk::WriteDescriptorSet::default()
        .dst_set(descriptor_set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(std::slice::from_ref(&image_info));
    unsafe { device.device.update_descriptor_sets(&[write], &[]) };
}

/// `opaque`: this draw is inside the surface's declared opaque region, so it may
/// replace the destination rather than blend over it.
pub fn draw(
    device: &VulkanDevice,
    pipelines: &CompositePipelines,
    cmd: vk::CommandBuffer,
    descriptor_set: vk::DescriptorSet,
    push: PushQuad,
    opaque: bool,
) {
    composite::draw_textured(device, pipelines, cmd, descriptor_set, push, opaque);
}
