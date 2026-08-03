//! The two image barriers around a worker render pass, and the queue-family
//! ownership rules behind them.
//!
//! # Why there is a release but no acquire
//!
//! The pass RELEASES the image to `QUEUE_FAMILY_FOREIGN_EXT` and never acquires
//! it back. That is the standard dmabuf producer idiom, not an oversight:
//! Vulkan requires an acquire only when the new owner needs the PREVIOUS
//! contents, and the pass clears and fully redraws the slot — which is exactly
//! what entering from `oldLayout = UNDEFINED` states. The release pairs with the
//! COMPOSITOR's acquire (`record_pending_acquires`, src `FOREIGN`), and that is
//! where the handoff actually matters: the worker and the compositor are
//! separate `VkDevice`s.
//!
//! # Why skipping it on some drivers is safe
//!
//! The release is skipped without `VK_EXT_queue_family_foreign` — `multiplane`
//! IS that extension probe (see `device.factory`) — because the barrier cannot
//! be expressed without it.
//!
//! NOT because such a driver is untiled. V3D on the Pi exposes `BROADCOM_UIF`,
//! and the bridge ranks tiled above linear, so a Pi really is rendering into a
//! tiled buffer here. What the acquire buys is resolving AUXILIARY COMPRESSION
//! metadata (AMD DCC and kin) before another engine reads. Plain tiling needs no
//! resolve: the layout travels in the DRM modifier, both ends negotiated the
//! same one, and cross-device visibility is the dmabuf's own business. So the
//! drivers that may skip this are exactly those with no aux plane — which is the
//! set that lacks the extension. A driver that compresses but lacks it would
//! sample garbage; `probe_multiplane` warns loudly because none is known.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;

const SUBRESOURCE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// Enter the pass: contents are fully replaced, so this comes from UNDEFINED and
/// performs no ownership acquire.
pub fn enter(dev: &VulkanDevice, cmd: vk::CommandBuffer, image: vk::Image) {
    record(
        dev, cmd, image,
        vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        false,
    );
}

/// Leave the pass: release to the foreign queue family for the compositor.
pub fn leave(dev: &VulkanDevice, cmd: vk::CommandBuffer, image: vk::Image) {
    record(
        dev, cmd, image,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::GENERAL,
        vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
        vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::empty(),
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn record(
    d: &VulkanDevice, cmd: vk::CommandBuffer, image: vk::Image,
    old: vk::ImageLayout, new: vk::ImageLayout,
    src_stage: vk::PipelineStageFlags2, dst_stage: vk::PipelineStageFlags2,
    src_access: vk::AccessFlags2, dst_access: vk::AccessFlags2, release_foreign: bool,
) {
    let mut b = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(src_stage).dst_stage_mask(dst_stage)
        .src_access_mask(src_access).dst_access_mask(dst_access)
        .old_layout(old).new_layout(new)
        .image(image).subresource_range(SUBRESOURCE);
    if release_foreign && d.multiplane {
        b = b.src_queue_family_index(d.queue_family_index)
            .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT);
    }
    let dep = vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&b));
    unsafe { d.device.cmd_pipeline_barrier2(cmd, &dep) };
}
