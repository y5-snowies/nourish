//! Exportable render-target creation + VkImage -> dmabuf export (the scanout
//! handoff). Phase 4 Step 3 — real for single-plane modifiers (what
//! `format.modifier` negotiates for the offered ARGB/ABGR formats).

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::vulkan::PhysicalDevice;
use std::os::unix::io::{FromRawFd, OwnedFd};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("unsupported fourcc for the vulkan path: {0:?}")]
    UnsupportedFormat(Fourcc),
    #[error("vulkan call failed: {0}")]
    Vk(String),
    #[error("dmabuf assembly failed")]
    Assembly,
}

pub struct ExportableImage {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub fourcc: Fourcc,
    pub size: (u32, u32),
}

impl ExportableImage {
    /// Destroy the backing VkImage/memory/view. Safe to call once the image is
    /// no longer rendered into — the exported [`Dmabuf`] is a standalone kernel
    /// object (its fd is a dup of the memory) and remains valid afterwards.
    ///
    /// `create_exportable` callers that only need the exported dmabuf MUST call
    /// this after [`export`]; otherwise the VkImage + dedicated VkDeviceMemory
    /// leak (a full-screen allocation per call — e.g. on every output resize).
    pub fn destroy(self, device: &VulkanDevice) {
        unsafe {
            device.device.destroy_image_view(self.view, None);
            device.device.destroy_image(self.image, None);
            device.device.free_memory(self.memory, None);
        }
    }
}

/// Create a render target whose memory can be exported as a dmabuf, using a
/// driver-selected modifier from the negotiated list.
pub fn create_exportable(
    device: &VulkanDevice,
    phd: &PhysicalDevice,
    fourcc: Fourcc,
    size: (u32, u32),
    modifiers: &[Modifier],
) -> Result<ExportableImage, ExportError> {
    let format = compositor_kernel_vulkan_format_query_base::query::vk_format(fourcc)
        .ok_or(ExportError::UnsupportedFormat(fourcc))?;
    let _ = phd;

    let modifier_list: Vec<u64> = modifiers.iter().map(|m| Into::<u64>::into(*m)).collect();
    let mut modifier_info = vk::ImageDrmFormatModifierListCreateInfoEXT::default()
        .drm_format_modifiers(&modifier_list);
    let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width: size.0,
            height: size.1,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut modifier_info)
        .push_next(&mut external_info);

    let dev = &device.device;
    let image = unsafe {
        dev.create_image(&image_info, None)
            .map_err(|e| ExportError::Vk(format!("create_image: {e}")))?
    };

    let requirements = unsafe { dev.get_image_memory_requirements(image) };
    let mut export_info = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(requirements.memory_type_bits.trailing_zeros())
        .push_next(&mut export_info)
        .push_next(&mut dedicated);

    let memory = unsafe {
        dev.allocate_memory(&alloc_info, None).map_err(|e| {
            dev.destroy_image(image, None);
            ExportError::Vk(format!("allocate_memory: {e}"))
        })?
    };
    unsafe {
        dev.bind_image_memory(image, memory, 0).map_err(|e| {
            dev.free_memory(memory, None);
            dev.destroy_image(image, None);
            ExportError::Vk(format!("bind_image_memory: {e}"))
        })?;
    }

    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    let view = unsafe {
        dev.create_image_view(&view_info, None).map_err(|e| {
            dev.free_memory(memory, None);
            dev.destroy_image(image, None);
            ExportError::Vk(format!("create_image_view: {e}"))
        })?
    };

    Ok(ExportableImage {
        image,
        memory,
        view,
        format,
        fourcc,
        size,
    })
}

/// Export the image as a dmabuf: memory fd + driver-reported modifier +
/// plane-0 subresource layout.
pub fn export(device: &VulkanDevice, img: &ExportableImage) -> Result<Dmabuf, ExportError> {
    let dev = &device.device;

    // Which modifier did the driver pick?
    let modifier_loader =
        ash::ext::image_drm_format_modifier::Device::new(&device.instance, dev);
    let mut props = vk::ImageDrmFormatModifierPropertiesEXT::default();
    unsafe {
        modifier_loader
            .get_image_drm_format_modifier_properties(img.image, &mut props)
            .map_err(|e| ExportError::Vk(format!("get modifier: {e}")))?;
    }
    let modifier = Modifier::from(props.drm_format_modifier);

    // Plane-0 layout for the chosen modifier.
    let subresource = vk::ImageSubresource {
        aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
        mip_level: 0,
        array_layer: 0,
    };
    let layout = unsafe { dev.get_image_subresource_layout(img.image, subresource) };

    // The memory fd (one per plane reference; single-plane formats here).
    let fd_loader = ash::khr::external_memory_fd::Device::new(&device.instance, dev);
    let get_info = vk::MemoryGetFdInfoKHR::default()
        .memory(img.memory)
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let raw = unsafe {
        fd_loader
            .get_memory_fd(&get_info)
            .map_err(|e| ExportError::Vk(format!("get_memory_fd: {e}")))?
    };
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };

    let mut builder = Dmabuf::builder(
        (img.size.0 as i32, img.size.1 as i32),
        img.fourcc,
        modifier,
        DmabufFlags::empty(),
    );
    builder.add_plane(fd, layout.offset as u32, layout.row_pitch as u32);
    builder.build().ok_or(ExportError::Assembly)
}
