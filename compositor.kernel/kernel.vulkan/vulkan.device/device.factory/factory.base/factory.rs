//! Logical VkDevice creation with the compositor's required extensions and
//! Vulkan 1.3 core features. Feature policy: timeline semaphores (1.2 core),
//! dynamic rendering + synchronization2 (1.3 core); promoted
//! VK_KHR_timeline_semaphore is NOT requested.

use ash::vk;
use smithay::backend::vulkan::PhysicalDevice;
use std::ffi::{CStr, c_char};

/// Device extensions the render path CANNOT work without — absence is fatal.
pub fn required_extensions() -> Vec<&'static CStr> {
    vec![
        ash::khr::external_memory_fd::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::ext::image_drm_format_modifier::NAME,
        ash::khr::external_semaphore_fd::NAME,
    ]
}

/// Gate for Intel-CCS-style multi-plane dmabuf support: disjoint multi-plane import plus
/// the `VK_QUEUE_FAMILY_FOREIGN_EXT` acquire that makes the driver flush/decompress the
/// aux plane. Read across the vulkan import path via [`VulkanDevice::multiplane`].
///
/// This is PROBED, not assumed. It used to be a hardcoded `true` that pushed
/// `VK_EXT_queue_family_foreign` into [`required_extensions`], which made a driver
/// lacking that extension fail `create` outright — no Vulkan renderer at all, on hardware
/// whose only shortcoming is that it has no compressed formats to import in the first
/// place. Broadcom V3D (Raspberry Pi) is exactly that case. Probing turns a dead renderer
/// into a working single-plane one and takes nothing away from devices that do have it.
///
/// `has_device_extension` is a direct `vkEnumerateDeviceExtensionProperties` answer — a
/// definitive per-device fact, not a vendor heuristic. It settles only whether the
/// EXTENSION exists; whether a given format is actually disjoint-importable stays a
/// per-format question, answered where it already was (the `DISJOINT`/plane-count checks
/// at the format-query and import layers).
fn probe_multiplane(phd: &PhysicalDevice) -> bool {
    let ok = phd.has_device_extension(ash::ext::queue_family_foreign::NAME);
    match ok {
        true => info!("vulkan: multi-plane (VK_EXT_queue_family_foreign) available"),
        // WARN, not info: this branch is where probing is strictly worse than the old hard
        // failure, and it should be visible. A disjoint import still fails loudly
        // (`ImportError::Disjoint`), but the FOREIGN-QUEUE ACQUIRE is silently skipped — and
        // that acquire is what makes a driver flush/decompress tiled or DCC content before we
        // sample it. On a device that produces compressed buffers but somehow lacks this
        // extension, the result would be wrong pixels rather than an error. No such driver is
        // known (compression-capable drivers all expose it; the ones that don't, like
        // Broadcom V3D, have nothing to decompress), so this is a warning about a
        // hypothetical — but a silent wrong-pixels path deserves a line in the log.
        false => warn!(
            "vulkan: VK_EXT_queue_family_foreign absent — single-plane dmabuf import only, \
             and foreign-queue acquires are skipped. Expected on drivers without compressed \
             formats (e.g. V3D); on an AMD or Intel GPU this is unexpected — report it."
        ),
    }
    ok
}

pub struct VulkanDevice {
    pub device: ash::Device,
    pub queue_family_index: u32,
    /// Whether `VK_EXT_queue_family_foreign` was available and enabled — see
    /// [`probe_multiplane`]. False means every import takes the single-plane path.
    pub multiplane: bool,
    /// The owning `ash::Instance`, retained so device-level extension loaders
    /// (`ash::khr/ext::*::Device::new`) can be constructed — they need the
    /// instance to resolve `vkGetDeviceProcAddr`.
    pub instance: ash::Instance,
    /// Whether the descriptor-indexing features needed for a bindless
    /// `binding_array<texture>` (runtime array + partially-bound + non-uniform
    /// sampled-image indexing) were advertised AND enabled. Probed at creation;
    /// gates the `window-textures` engine interface (bundles that need it fall
    /// back when this is false — no device-creation risk on adapters lacking it).
    pub descriptor_indexing: bool,
    /// Whether a FRAGMENT shader may write to a storage buffer.
    ///
    /// Core Vulkan 1.0 and near-universal, but off unless asked for — and a
    /// shader that stores without it is undefined rather than an error, so it is
    /// probed and enabled the same way descriptor indexing is, and a bundle
    /// declaring `storage` is refused where it is absent.
    pub storage_writes: bool,
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
    let multiplane = probe_multiplane(phd);

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
    // Enable the optional extension only when the driver has it; requesting an absent
    // extension is a `vkCreateDevice` failure, so this list must match the probe.
    let mut enabled = required_extensions();
    if multiplane {
        enabled.push(ash::ext::queue_family_foreign::NAME);
    }
    let ext_ptrs: Vec<*const c_char> = enabled.iter().map(|e| e.as_ptr()).collect();

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
    // Storage writes from the fragment stage, for bundles that declare `storage`.
    // Enabling an unsupported feature fails `vkCreateDevice` outright — which
    // would black-screen the compositor — so this is advertised-or-nothing, like
    // every other optional feature here.
    let storage_writes = supported.fragment_stores_and_atomics == vk::TRUE;
    if storage_writes {
        base_features = base_features.fragment_stores_and_atomics(true);
    }

    // Probe descriptor-indexing (Vulkan 1.2) for a bindless window-texture array.
    // Query support via features2, then enable ONLY if all three needed bits are
    // advertised — enabling an unsupported feature fails vkCreateDevice (would
    // black-screen the compositor), so an adapter without them simply reports the
    // capability as false and `window_textures` bundles fall back.
    //
    // NOT gated on which bundle is selected. This factory builds BOTH the
    // compositor's device and the background worker's second device, and
    // `vkCreateDevice` happens once per device while the selection changes whenever
    // the user picks a shader — so there is no bundle-shaped answer to give here,
    // and the flag means "what this GPU can do", which a preference has no business
    // overriding.
    let descriptor_indexing = {
        let mut probe = vk::PhysicalDeviceVulkan12Features::default();
        let mut f2 = vk::PhysicalDeviceFeatures2::default().push_next(&mut probe);
        unsafe { instance.get_physical_device_features2(phd.handle(), &mut f2) };
        probe.runtime_descriptor_array == vk::TRUE
            && probe.descriptor_binding_partially_bound == vk::TRUE
            && probe.shader_sampled_image_array_non_uniform_indexing == vk::TRUE
    };
    if descriptor_indexing {
        features12 = features12
            .runtime_descriptor_array(true)
            .descriptor_binding_partially_bound(true)
            .shader_sampled_image_array_non_uniform_indexing(true);
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

    info!(
        "vulkan logical device created (queue family {queue_family_index}, \
         descriptor_indexing={descriptor_indexing}, storage_writes={storage_writes})"
    );
    Ok(VulkanDevice {
        device,
        queue_family_index,
        multiplane,
        instance: instance.clone(),
        descriptor_indexing,
        storage_writes,
    })
}
