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
//! Alpha is required — the pass clears to transparent black and blends
//! premultiplied-over, and floating panes composite the result — so every rung
//! carries it, and the opaque `X*` codes the scanout ladder prefers are not
//! candidates here.
//!
//! # Channel order follows the session, it is not hardcoded
//!
//! Both orders are equally renderable on the devices measured (NVIDIA reports
//! `COLOR_ATTACHMENT` for `A2B10G10R10` *and* `A2R10G10B10`, 6 modifiers each), so
//! this is not about capability. It is about not diverging from the scanout for no
//! reason: KMS planes expose 10-bit in ONE order on some hardware (NVIDIA offers
//! `AB30`/`XB30` and no `AR30`/`XR30`), and where this buffer can be promoted
//! straight to a plane, only the order the plane exposes can take that path. A
//! hardcoded order also silently costs the whole feature on a device that renders
//! only the other one — the failure is a drop to the 8-bit rung, on the surface
//! where banding shows most.
//!
//! So the ladder is: the session's own order first, the other order second, then
//! the 8-bit floor. See `developer.tool.color/probe-change.MD` for how a hardcoded
//! order hid a permanently-8-bit session.

use smithay::backend::allocator::Fourcc;
use smithay::backend::vulkan::PhysicalDevice;

/// Can this device render into `fourcc` AND hand it out as a dmabuf?
fn usable(phd: &PhysicalDevice, fourcc: Fourcc) -> bool {
    let Some(vk) = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc) else {
        return false;
    };
    compositor_kernel_vulkan_format_query_base::query::renderable(phd, vk)
        && !compositor_kernel_vulkan_format_modifier_base::modifier::modifiers(phd, vk).is_empty()
}

/// The best format this worker can actually produce for this session.
pub fn select(formats: &compositor_kernel_graphic_format_registrar_base::registrar::Registrar, phd: &PhysicalDevice) -> Fourcc {
    use compositor_kernel_graphic_format_catalog_base::catalog;
    let session = formats.scanout_fourcc();
    let deep = session.is_some_and(catalog::is_deep);

    // WHICH fourccs are worth trying comes from the format layer; whether THIS
    // device can render one stays here, where the PhysicalDevice is.
    for candidate in catalog::background_ladder(session, deep) {
        if usable(phd, candidate) {
            info!(
                "background worker: rendering {candidate:?} (session deep={deep}, scanout={session:?})"
            );
            return candidate;
        }
        warn!("background worker: {candidate:?} unusable on this device; trying the next");
    }
    // Nothing in the ladder works. Return the floor anyway so the caller fails at
    // allocation with a concrete Vulkan error rather than on a silent guess.
    compositor_kernel_graphic_format_catalog_base::catalog::FLOOR
}
