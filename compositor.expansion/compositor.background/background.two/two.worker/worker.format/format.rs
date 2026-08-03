//! What pixel format the background renders into.
//!
//! Unlike the UI producers, the background is not pinned to 8-bit by anything.
//! iced and bevy render through wgpu at a fixed `Bgra8UnormSrgb`, so `Argb8888`
//! is the only fourcc that reinterprets correctly for them. The background owns
//! its whole pipeline — raw ash, its own SPIR-V, its own `VkDevice` — and the
//! shader writes `vec4<f32>` whatever the attachment is, so it can match the
//! session.
//!
//! It SHOULD match, and this is the one surface where it matters most. A
//! backdrop is a large, smooth, low-contrast gradient — the most banding-prone
//! thing on screen — so rendering it at 8 bits and upconverting into a 10-bit
//! scanout spends the extra bits exactly where they do nothing.
//!
//! Three conditions, all of them already answered elsewhere:
//!
//! * the SESSION is deep — taken from the fourcc the swapchain actually got, not
//!   from the requested depth, because the achieved format already folds in
//!   "deep colour was asked for", "the plane accepted it" and "a mode was found";
//! * the worker's device can COLOR-ATTACH the format;
//! * it can EXPORT it — at least one DRM modifier, or `create_exportable` has
//!   nothing to offer the driver.
//!
//! Alpha is required, so the ladder is `Argb2101010` and not the `Xrgb2101010`
//! the scanout ladder prefers: the pass clears to transparent black and blends
//! premultiplied-over, and floating panes composite the result.

use smithay::backend::allocator::Fourcc;
use smithay::backend::vulkan::PhysicalDevice;

/// Deep first, then the 8-bit floor. Both carry alpha.
const LADDER: [Fourcc; 2] = [Fourcc::Argb2101010, Fourcc::Argb8888];

/// Can this device render into `fourcc` AND hand it out as a dmabuf?
fn usable(phd: &PhysicalDevice, fourcc: Fourcc) -> bool {
    let Some(vk) = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc) else {
        return false;
    };
    compositor_kernel_vulkan_format_query_base::query::renderable(phd, vk)
        && !compositor_kernel_vulkan_format_modifier_base::modifier::modifiers(phd, vk).is_empty()
}

/// The best format this worker can actually produce for this session.
pub fn select(phd: &PhysicalDevice) -> Fourcc {
    let deep = compositor_kernel_graphic_bridge_negotiate_compositor::compositor::scanout_is_deep();
    for candidate in LADDER {
        if candidate != Fourcc::Argb8888 && !deep {
            continue;
        }
        if usable(phd, candidate) {
            info!("background worker: rendering {candidate:?} (session deep={deep})");
            return candidate;
        }
        warn!("background worker: {candidate:?} unusable on this device; trying the next");
    }
    // Nothing in the ladder works. Return the floor anyway so the caller fails at
    // allocation with a concrete Vulkan error rather than on a silent guess.
    Fourcc::Argb8888
}
