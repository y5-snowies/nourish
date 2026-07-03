//! Enumerate the DRM format modifiers a wgpu (Vulkan) adapter can import for the
//! bridge's candidate fourccs, via raw `ash` on the adapter's VkPhysicalDevice —
//! wgpu itself exposes no modifier enumeration. Returns a smithay `FormatSet` the
//! startup intersection consumes. Multi-plane (AMD DCC / Intel CCS) modifiers are
//! dropped unless `allow_dcc`, since the single-plane wgpu HAL import can't take them.

use ash::vk;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};
use wgpu::hal::api::Vulkan;

/// Bridge candidate fourccs → the VkFormat used to import them. ARGB2101010 is
/// omitted deliberately: it has no wgpu `TextureFormat`.
const CANDIDATES: &[(Fourcc, vk::Format)] = &[
    (Fourcc::Argb8888, vk::Format::B8G8R8A8_UNORM),
    (Fourcc::Xrgb8888, vk::Format::B8G8R8A8_UNORM),
    (Fourcc::Abgr8888, vk::Format::R8G8B8A8_UNORM),
    (Fourcc::Xbgr8888, vk::Format::R8G8B8A8_UNORM),
    (Fourcc::Abgr2101010, vk::Format::A2B10G10R10_UNORM_PACK32),
];

/// Usage the bridge textures are imported with — the basis for the `probe` check.
const BRIDGE_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw()
        | vk::ImageUsageFlags::SAMPLED.as_raw()
        | vk::ImageUsageFlags::TRANSFER_SRC.as_raw()
        | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);

/// The `(fourcc × modifier)` set `adapter` can import as a color target. Empty if
/// the adapter is not a Vulkan backend. `allow_dcc` keeps multi-plane modifiers;
/// `probe` additionally verifies each modifier is usable for the real image usage.
pub fn query_importable(
    instance: &wgpu::Instance,
    adapter: &wgpu::Adapter,
    allow_dcc: bool,
    probe: bool,
) -> FormatSet {
    let mut out: Vec<DrmFormat> = Vec::new();
    unsafe {
        let Some(hal_instance) = instance.as_hal::<Vulkan>() else {
            return FormatSet::default();
        };
        let ash_instance = hal_instance.shared_instance().raw_instance();
        let Some(hal_adapter) = adapter.as_hal::<Vulkan>() else {
            return FormatSet::default();
        };
        let phd = hal_adapter.raw_physical_device();

        for &(fourcc, vk_format) in CANDIDATES {
            for modifier in modifiers_for(ash_instance, phd, vk_format, allow_dcc, probe) {
                out.push(DrmFormat {
                    code: fourcc,
                    modifier,
                });
            }
        }
    }
    out.into_iter().collect()
}

/// Pick the Vulkan adapter whose DRM **render** node matches `render_node`
/// (via `VK_EXT_physical_device_drm`). Pinning is the default (opt out with
/// `gpu_no_pin_wgpu_node`). `None` ⇒ no match (caller falls back to the default adapter).
pub fn pick_adapter(instance: &wgpu::Instance, render_node: &str) -> Option<wgpu::Adapter> {
    let (want_major, want_minor) = node_rdev(render_node)?;
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    for adapter in adapters {
        let matches = unsafe {
            let Some(hal_instance) = instance.as_hal::<Vulkan>() else {
                continue;
            };
            let ash_instance = hal_instance.shared_instance().raw_instance();
            let Some(hal_adapter) = adapter.as_hal::<Vulkan>() else {
                continue;
            };
            let phd = hal_adapter.raw_physical_device();
            let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
            let mut p2 = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
            ash_instance.get_physical_device_properties2(phd, &mut p2);
            drm.has_render != 0
                && drm.render_major as u64 == want_major
                && drm.render_minor as u64 == want_minor
        };
        if matches {
            return Some(adapter);
        }
    }
    None
}

/// `(major, minor)` of a DRM node's device id, via `stat(2)`.
fn node_rdev(path: &str) -> Option<(u64, u64)> {
    let cpath = std::ffi::CString::new(path).ok()?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::stat(cpath.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some((libc::major(st.st_rdev) as u64, libc::minor(st.st_rdev) as u64))
}

/// Empirical probe: can a DMA-buf image actually be created for this format+modifier
/// with the bridge usage? Stronger than the format-features check (catches
/// advertised-but-unusable modifiers). Runs only under `gpu_probe_modifiers`.
unsafe fn image_creatable(
    instance: &ash::Instance,
    phd: vk::PhysicalDevice,
    format: vk::Format,
    modifier: u64,
) -> bool {
    let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut ext = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(BRIDGE_USAGE)
        .push_next(&mut drm)
        .push_next(&mut ext);
    let mut props = vk::ImageFormatProperties2::default();
    unsafe {
        instance
            .get_physical_device_image_format_properties2(phd, &info, &mut props)
            .is_ok()
    }
}

/// Importable modifiers for one VkFormat (two-call enumerate pattern), filtered to
/// color-attachment-capable and (unless `allow_dcc`) single-plane; when `probe`,
/// each survivor is verified image-creatable for the real usage.
unsafe fn modifiers_for(
    instance: &ash::Instance,
    phd: vk::PhysicalDevice,
    format: vk::Format,
    allow_dcc: bool,
    probe: bool,
) -> Vec<Modifier> {
    // Pass 1: count.
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
    let mut props = vk::FormatProperties2::default().push_next(&mut list);
    unsafe { instance.get_physical_device_format_properties2(phd, format, &mut props) };
    let count = list.drm_format_modifier_count as usize;
    if count == 0 {
        return Vec::new();
    }

    // Pass 2: fill.
    let mut storage = vec![vk::DrmFormatModifierPropertiesEXT::default(); count];
    let mut list =
        vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut storage);
    let mut props = vk::FormatProperties2::default().push_next(&mut list);
    unsafe { instance.get_physical_device_format_properties2(phd, format, &mut props) };

    storage
        .iter()
        .filter(|p| {
            p.drm_format_modifier_tiling_features
                .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
                && (allow_dcc || p.drm_format_modifier_plane_count <= 1)
                && (!probe
                    || unsafe { image_creatable(instance, phd, format, p.drm_format_modifier) })
        })
        .map(|p| Modifier::from(p.drm_format_modifier))
        .collect()
}
