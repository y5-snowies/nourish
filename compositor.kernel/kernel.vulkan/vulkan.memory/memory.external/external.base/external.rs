//! Sharing an image between two logical devices on the same PHYSICAL device,
//! via `OPAQUE_FD` external memory.
//!
//! This is how a SHM window reaches the background worker. A `wl_shm` surface has
//! no dmabuf — the compositor uploaded its pixels into a device-local image — so
//! there is no client fd to pass on the way there is for a dmabuf client.
//!
//! # Why OPAQUE_FD rather than a dmabuf
//!
//! Exporting as a dmabuf would drag in `VK_EXT_image_drm_format_modifier` and, in
//! practice, LINEAR tiling — a sampling cost paid by every SHM surface whether or
//! not any shader wants it. `OPAQUE_FD` has no such constraint: it keeps OPTIMAL
//! tiling, needs no modifier negotiation, and costs one allocation flag.
//!
//! Its restriction is that both ends must be the same physical device, which is
//! exactly the case here: `worker.physical` selects the compositor's render node
//! deliberately and with NO fallback, because the worker's output has to be
//! importable by the compositor anyway. Two logical devices, one GPU.
//!
//! The image is recreated on the far side from the same parameters and bound to
//! the imported memory, so both sides must agree on them — which is why [`Shared`]
//! carries them rather than assuming a convention.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};

/// Everything a second device needs to reconstruct an image over shared memory.
/// Holds the fd, so dropping it closes the handle; wrap in an `Arc` to share.
#[derive(Debug)]
pub struct Shared {
    pub fd: OwnedFd,
    pub width: u32,
    pub height: u32,
    pub format: vk::Format,
    /// `VkMemoryRequirements::size` of the exporting allocation. The importer must
    /// allocate exactly this, not its own requirement, or the bind is invalid.
    pub size: u64,
}

/// The image create-info both sides must build identically. Exposed so the
/// exporter creates an image that CAN be shared and the importer recreates the
/// same one — a mismatch here is undefined behaviour, not an error.
///
/// `external` is optional so the ORDINARY (unshared) SHM upload builds exactly the
/// create-info it built before this crate existed. That is the point: an image
/// nothing will export must not carry external-memory flags, which can change the
/// driver's memory-type bits, alignment and allocation size.
pub fn image_info<'a>(
    format: vk::Format,
    width: u32,
    height: u32,
    external: Option<&'a mut vk::ExternalMemoryImageCreateInfo<'_>>,
) -> vk::ImageCreateInfo<'a> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width, height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    match external {
        Some(e) => info.push_next(e),
        None => info,
    }
}

/// Export `memory` as an `OPAQUE_FD`. The allocation must have been made with
/// `VkExportMemoryAllocateInfo` naming the same handle type.
pub fn export(dev: &VulkanDevice, memory: vk::DeviceMemory, size: u64,
              format: vk::Format, width: u32, height: u32) -> Result<Shared, VulkanError> {
    let loader = ash::khr::external_memory_fd::Device::new(&dev.instance, &dev.device);
    let raw = unsafe {
        loader
            .get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD),
            )
            .map_err(|e| VulkanError::Vk(format!("export opaque fd: {e}")))?
    };
    Ok(Shared { fd: unsafe { OwnedFd::from_raw_fd(raw) }, width, height, format, size })
}

/// Recreate `s` on this device. The fd is DUPLICATED, so the imported image
/// outlives the exporter's copy and neither side owns the other's lifetime.
pub fn import(
    dev: &VulkanDevice,
    s: &Shared,
) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), VulkanError> {
    let d = &dev.device;
    let mut ext = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let image = unsafe {
        d.create_image(&image_info(s.format, s.width, s.height, Some(&mut ext)), None)
            .map_err(|e| VulkanError::Vk(format!("external image: {e}")))?
    };
    let req = unsafe { d.get_image_memory_requirements(image) };
    let idx = (0..32)
        .find(|i| req.memory_type_bits & (1 << i) != 0)
        .ok_or_else(|| VulkanError::Vk("external: no memory type".into()))?;
    // dup: vkImportMemoryFdKHR takes ownership of the fd it is given.
    let dup = s.fd.try_clone().map_err(|e| VulkanError::Vk(format!("dup: {e}")))?;
    let mut import = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD)
        .fd(dup.as_raw_fd());
    let memory = unsafe {
        d.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(s.size)
                .memory_type_index(idx)
                .push_next(&mut import),
            None,
        )
        .inspect_err(|_| d.destroy_image(image, None))
        .map_err(|e| VulkanError::Vk(format!("external memory: {e}")))?
    };
    std::mem::forget(dup); // consumed by the driver on success
    unsafe {
        d.bind_image_memory(image, memory, 0)
            .map_err(|e| VulkanError::Vk(format!("external bind: {e}")))?;
    }
    let view = unsafe {
        d.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(s.format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                }),
            None,
        )
        .map_err(|e| VulkanError::Vk(format!("external view: {e}")))?
    };
    Ok((image, memory, view))
}
