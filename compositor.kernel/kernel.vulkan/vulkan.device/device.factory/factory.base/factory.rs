//! Logical VkDevice creation with the compositor's required extensions and
//! Vulkan 1.3 core features. Feature policy: timeline semaphores (1.2 core),
//! dynamic rendering + synchronization2 (1.3 core); promoted
//! VK_KHR_timeline_semaphore is NOT requested.

use ash::vk;
use smithay::backend::vulkan::PhysicalDevice;
use std::ffi::{CStr, c_char};

/// MASTER GATE for Intel-CCS / multi-plane dmabuf support: disjoint multi-plane
/// import + the VK_QUEUE_FAMILY_FOREIGN_EXT acquire that samples the compressed
/// planes. Read across the vulkan import path. Off = single-plane-only import.
pub const MULTIPLANE_SUPPORT: bool = true;

/// Device extensions the render path requires.
pub fn required_extensions() -> Vec<&'static CStr> {
    let mut ext = vec![
        ash::khr::external_memory_fd::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::ext::image_drm_format_modifier::NAME,
        ash::khr::external_semaphore_fd::NAME,
    ];
    if MULTIPLANE_SUPPORT {
        ext.push(ash::ext::queue_family_foreign::NAME);
    }
    ext
}

pub struct VulkanDevice {
    pub device: ash::Device,
    pub queue_family_index: u32,
    /// The owning `ash::Instance`, retained so device-level extension loaders
    /// (`ash::khr/ext::*::Device::new`) can be constructed — they need the
    /// instance to resolve `vkGetDeviceProcAddr`.
    pub instance: ash::Instance,
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("no graphics queue family")]
    NoGraphicsQueue,
    #[error("missing required extension: {0}")]
    MissingExtension(String),
    #[error("vkCreateDevice failed: {0}")]
    Create(String),
}

pub fn create(phd: &PhysicalDevice) -> Result<VulkanDevice, DeviceError> {
    for ext in required_extensions() {
        if !phd.has_device_extension(ext) {
            return Err(DeviceError::MissingExtension(
                ext.to_string_lossy().into_owned(),
            ));
        }
    }

    let instance = phd.instance().handle();
    let queue_family_index = unsafe {
        instance
            .get_physical_device_queue_family_properties(phd.handle())
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or(DeviceError::NoGraphicsQueue)? as u32
    };

    let priorities = [1.0f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);
    let ext_ptrs: Vec<*const c_char> = required_extensions().iter().map(|e| e.as_ptr()).collect();

    let mut features12 =
        vk::PhysicalDeviceVulkan12Features::default().timeline_semaphore(true);
    let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
        .dynamic_rendering(true)
        .synchronization2(true);

    // Enable anisotropic sampling when the device advertises it, so the world anti-aliasing
    // `aniso` composite sampler is legal. No cost when unused; skipped on the
    // rare device that lacks it (the composite path falls back to isotropic).
    let supported = unsafe { instance.get_physical_device_features(phd.handle()) };
    let mut base_features = vk::PhysicalDeviceFeatures::default();
    if supported.sampler_anisotropy == vk::TRUE {
        base_features = base_features.sampler_anisotropy(true);
    }

    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(std::slice::from_ref(&queue_info))
        .enabled_extension_names(&ext_ptrs)
        .enabled_features(&base_features)
        .push_next(&mut features12)
        .push_next(&mut features13);

    let device = unsafe {
        instance
            .create_device(phd.handle(), &create_info, None)
            .map_err(|e| DeviceError::Create(format!("{e}")))?
    };

    info!("vulkan logical device created (queue family {queue_family_index})");
    Ok(VulkanDevice {
        device,
        queue_family_index,
        instance: instance.clone(),
    })
}
