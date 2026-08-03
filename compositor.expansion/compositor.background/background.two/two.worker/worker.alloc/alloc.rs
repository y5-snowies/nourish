//! Allocate the worker's render targets on the render node, through its own
//! Vulkan device.
//!
//! The modifier list IS queried — `create_exportable` is handed every DRM format
//! modifier the physical device reports for this format and the driver picks
//! from it, so these are never implicit/INVALID. What is skipped is the
//! CROSS-API intersection the gbm bridge does (gles ∩ wgpu), and skipping it is
//! sound only because both ends are the same Vulkan implementation on the same
//! device: `worker.device` matches the compositor's configured render node, so
//! anything this device can render into it can also import.

use compositor_background_two_worker_target::target::Target;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use smithay::backend::allocator::Fourcc;
use smithay::backend::vulkan::PhysicalDevice;

/// Build one render target.
pub fn allocate(
    dev: &VulkanDevice,
    phd: &PhysicalDevice,
    fourcc: Fourcc,
    size: (u32, u32),
) -> Result<Target, String> {
    let vk_fmt = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc)
        .ok_or_else(|| format!("unsupported fourcc {fourcc:?}"))?;
    let mods: Vec<_> =
        compositor_kernel_vulkan_format_modifier_base::modifier::modifiers(phd, vk_fmt)
            .into_iter()
            .map(|(m, _)| m)
            .collect();
    let img = compositor_kernel_vulkan_memory_export_base::export::create_exportable(
        dev, phd, fourcc, size, &mods,
    )
    .map_err(|e| format!("create_exportable: {e:?}"))?;
    let dmabuf = compositor_kernel_vulkan_memory_export_base::export::export(dev, &img)
        .map_err(|e| format!("export: {e:?}"))?;
    // What the DRIVER picked out of `mods`, which is the only interesting half.
    compositor_kernel_graphic_bridge_negotiate_report::report::allocation(
        "background worker", phd.name(), fourcc, mods.len(),
        smithay::backend::allocator::Buffer::format(&dmabuf).modifier,
    );
    // Keep the VkImage/memory/view alive: unlike the one-shot self-test that this
    // mirrors, the worker RENDERS into this image every frame. Only the dmabuf is
    // handed onward; both refer to one allocation.
    Ok(Target {
        image: img.image,
        memory: img.memory,
        view: img.view,
        format: img.format,
        dmabuf,
    })
}

/// Build a full ring of targets for one pane, `slots` deep.
pub fn allocate_slots(
    dev: &VulkanDevice,
    phd: &PhysicalDevice,
    fourcc: Fourcc,
    size: (u32, u32),
    slots: usize,
) -> Result<Vec<Target>, String> {
    let mut out = Vec::with_capacity(slots);
    for _ in 0..slots {
        out.push(allocate(dev, phd, fourcc, size)?);
    }
    Ok(out)
}
