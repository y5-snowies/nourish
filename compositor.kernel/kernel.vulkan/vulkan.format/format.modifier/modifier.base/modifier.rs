//! DRM-format-modifier negotiation — the vulkan side of explicit modifier
//! negotiation (the MODERN path; the Law-7 `framebuffer.modifier` fallback is
//! unrelated and stays off). Phase 4 Step 1 — real via smithay's
//! get_format_modifier_properties.

use ash::vk;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};
use smithay::backend::vulkan::PhysicalDevice;

/// All modifiers the device supports for a format, with their plane counts.
pub fn modifiers(phd: &PhysicalDevice, format: vk::Format) -> Vec<(Modifier, u32)> {
    phd.get_format_modifier_properties(format)
        .map(|props| {
            props
                .into_iter()
                .map(|p| {
                    (
                        Modifier::from(p.drm_format_modifier),
                        p.drm_format_modifier_plane_count,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Modifiers whose TILING FEATURES include `required` — the per-modifier answer,
/// which is the only binding one: a device can list a modifier for a format and
/// still not sample through it, and `VkFormatProperties` cannot say which.
fn modifiers_with(
    phd: &PhysicalDevice,
    format: vk::Format,
    required: vk::FormatFeatureFlags,
) -> Vec<Modifier> {
    phd.get_format_modifier_properties(format)
        .map(|props| {
            props
                .into_iter()
                .filter(|p| p.drm_format_modifier_tiling_features.contains(required))
                .map(|p| Modifier::from(p.drm_format_modifier))
                .collect()
        })
        .unwrap_or_default()
}

/// The (fourcc x modifier) set the vulkan renderer can IMPORT client buffers
/// with — every format in the exhaustive table that this device can sample,
/// through every modifier it can sample with.
///
/// This is what gets advertised to clients (intersected with the EGL set, which
/// still governs the legacy wl_drm path), and it is derived from the SAME table
/// `import_dmabuf` decides by. The two cannot disagree, which is the whole
/// point: a format advertised and not importable reaches the user as a blank
/// window, per-frame, with only a renderer warning to go on.
pub fn import_formats(phd: &PhysicalDevice) -> FormatSet {
    let mut formats: Vec<DrmFormat> = Vec::new();
    for (code, vk_fmt, _opaque) in compositor_kernel_vulkan_format_table_base::table::images() {
        if !compositor_kernel_vulkan_format_query_base::query::sampleable(phd, vk_fmt) {
            continue;
        }
        for modifier in modifiers_with(phd, vk_fmt, vk::FormatFeatureFlags::SAMPLED_IMAGE) {
            formats.push(DrmFormat { code, modifier });
        }
    }
    formats.into_iter().collect()
}

/// Build the render-format set (fourcc x modifier) the vulkan renderer
/// advertises — the input to scanout plane/format negotiation, mirroring what
/// the EGL context provides on the gles path.
pub fn render_formats(phd: &PhysicalDevice, fourccs: &[Fourcc]) -> FormatSet {
    let mut formats: Vec<DrmFormat> = Vec::new();
    for &code in fourccs {
        let Some(vk_fmt) = compositor_kernel_vulkan_format_query_base::query::vk_format(code) else {
            continue;
        };
        if !compositor_kernel_vulkan_format_query_base::query::renderable(phd, vk_fmt) {
            continue;
        }
        // PER-MODIFIER, mirroring `import_formats`: `renderable()` reads
        // `optimalTilingFeatures`, which answers for the FORMAT and says nothing about an
        // individual modifier. NVIDIA reports LINEAR with SAMPLED + TRANSFER/BLIT but
        // WITHOUT COLOR_ATTACHMENT (measured on a GTX 1050 and an RTX 4090 with
        // `developer.tool.color/color.format.stress/vkfmt`), so the unfiltered version
        // published a render target the device does not sanction — and on a cross-vendor
        // split, LINEAR is exactly what the scanout intersection then landed on. See
        // `~/PRIME_VULKAN.md`.
        for modifier in modifiers_with(phd, vk_fmt, vk::FormatFeatureFlags::COLOR_ATTACHMENT) {
            formats.push(DrmFormat { code, modifier });
        }
    }
    formats.into_iter().collect()
}
