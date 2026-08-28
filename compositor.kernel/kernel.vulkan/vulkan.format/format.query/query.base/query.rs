//! Renderable/texturable format discovery on a physical device, plus the thin
//! device-independent accessors over the fourcc table.
//!
//! The table lives in `format.table` and is EXHAUSTIVE over `Fourcc` — read its
//! module docs before adding a format anywhere. Nothing here may re-decide what
//! a fourcc maps to; every answer comes from `table::classify`.

use ash::vk;
use compositor_kernel_vulkan_format_table_base::table::{self, Vk};
use smithay::backend::allocator::Fourcc;
use smithay::backend::vulkan::PhysicalDevice;

/// Fourcc -> VkFormat, or `None` when the Vulkan path cannot take this format
/// (no such channel order, a Y/Cb/Cr format, or not a format at all).
pub fn vk_format(fourcc: Fourcc) -> Option<vk::Format> {
    match table::classify(fourcc) {
        Vk::Image { format, .. } => Some(format),
        Vk::Chroma | Vk::None(_) => None,
    }
}

/// Whether the fourth channel is undefined padding (an `X` format), so a
/// sampling view must swizzle alpha to `ONE`. Opaque formats share their
/// VkFormat with an alpha sibling, so the VkFormat alone cannot answer this —
/// which is why the import path asks here instead of keeping its own list.
pub fn opaque(fourcc: Fourcc) -> bool {
    matches!(table::classify(fourcc), Vk::Image { opaque: true, .. })
}

/// Why the Vulkan path refuses `fourcc` — for the log line that would otherwise
/// say only "unsupported".
pub fn refusal(fourcc: Fourcc) -> &'static str {
    match table::classify(fourcc) {
        Vk::Image { .. } => "supported",
        Vk::Chroma => "Y/Cb/Cr: needs a ycbcr-conversion sampler (not built)",
        Vk::None(why) => why,
    }
}

/// Whether the device can render to (color-attach) this format at all.
pub fn renderable(phd: &PhysicalDevice, format: vk::Format) -> bool {
    properties(phd, format)
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
}

/// Whether the device can SAMPLE this format — the honest gate for an import
/// list, which `renderable` is not: a client buffer is only ever sampled, and
/// requiring COLOR_ATTACHMENT drops formats (24bpp, single-channel) that import
/// and sample perfectly well. Either tiling counts; the per-modifier features
/// `format.modifier` applies are the stricter, final word.
pub fn sampleable(phd: &PhysicalDevice, format: vk::Format) -> bool {
    let props = properties(phd, format);
    (props.optimal_tiling_features | props.linear_tiling_features)
        .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
}

fn properties(phd: &PhysicalDevice, format: vk::Format) -> vk::FormatProperties {
    unsafe {
        phd.instance()
            .handle()
            .get_physical_device_format_properties(phd.handle(), format)
    }
}
