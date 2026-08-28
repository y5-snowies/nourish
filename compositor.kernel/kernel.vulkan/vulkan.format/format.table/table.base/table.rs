//! EVERY DRM fourcc a client can hand this compositor, classified for the
//! Vulkan import path — the one place that decides what the Vulkan renderer can
//! and cannot take.
//!
//! # Why exhaustive, and why that matters
//!
//! What clients are offered comes from EGL: smithay asks the driver
//! (`eglQueryDmaBufFormatsEXT`) and keeps everything `Fourcc::try_from`
//! recognises, so the offer is "whatever mesa lists", not a list of ours. The
//! Vulkan draw path then decided per-fourcc with a `_ => None` arm. A format the
//! driver listed and Vulkan had no arm for was therefore advertised, validated
//! (validation runs on GLES) and only then failed — at draw, every frame, as a
//! blank window plus `unsupported fourcc for the vulkan path`. That is how
//! `XB4H` (`Xbgr16161616f`, fp16) got through.
//!
//! So there is no wildcard arm here. [`classify`] matches every `Fourcc`
//! variant explicitly, and `Fourcc` (`drm_fourcc::DrmFourcc`) is NOT
//! `#[non_exhaustive]` — a variant added by a future drm-fourcc is a COMPILE
//! ERROR in this file until someone decides what Vulkan does with it. The same
//! list generates [`ALL`], so the set the renderer advertises cannot drift from
//! the set it can import: both are this table.
//!
//! # The mapping rules
//!
//! DRM names packed formats MSB→LSB of a little-endian value; Vulkan's `PACK16`
//! / `PACK32` formats do the same, so those names line up directly
//! (`Argb2101010` → `A2R10G10B10_UNORM_PACK32`). Vulkan's *unpacked* names are
//! in MEMORY order instead, which reverses them (`Argb8888` is B,G,R,A in
//! memory → `B8G8R8A8_UNORM`). Channel orders Vulkan simply has no format for
//! (`Bgra8888`, `Rgba8888`, `Argb16161616f`, `Rg88`) are [`Vk::None`], not a
//! near-miss: a wrong VkFormat here samples swapped channels.
//!
//! `X` formats map to their `A` sibling; the padding byte is undefined, so the
//! import view swizzles alpha to `ONE` (that is what `opaque` carries).
//!
//! Everything Y/Cb/Cr is [`Vk::Chroma`]: Vulkan can only describe those through
//! `VK_KHR_sampler_ycbcr_conversion` with a conversion-enabled sampler, which
//! this renderer does not build. They are a distinct answer from "no such
//! format" because the work to support them is known, not impossible.
//!
//! Transfer functions are not DECIDED here, only reported. `Abgr16161616f`
//! imports as `R16G16B16A16_SFLOAT`, and fp16 content is conventionally
//! linear-light / extended-range while the SDR composite passes sampled colour
//! through as if it were encoded (as does the GLES renderer — it has no transfer
//! handling either). Such formats carry `linear: true`, and whether the active
//! composite can express that is decided one layer up, in
//! `bridge.negotiate/negotiate.compositor`: only the HDR composite takes a
//! per-surface transfer, so in SDR those formats are neither advertised nor
//! accepted, and a client falls back to 8-bit sRGB — which IS expressed exactly.

use ash::vk;
use smithay::backend::allocator::Fourcc;

/// What the Vulkan path does with a fourcc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vk {
    /// Importable as a single-plane colour image. `opaque` = the fourth channel
    /// is undefined padding (an `X` format), so the view forces alpha to `ONE`.
    /// `linear` = the values are NOT sRGB-encoded by convention (the float
    /// formats: linear-light, extended range, may exceed 1.0 or go negative).
    /// Importability is a device fact; whether the COMPOSITE can express
    /// `linear` is a policy question, and `linear` is what lets the layer above
    /// ask it — see `bridge.negotiate/negotiate.compositor`.
    Image { format: vk::Format, opaque: bool, linear: bool },
    /// Y/Cb/Cr content: needs `VK_KHR_sampler_ycbcr_conversion`, which this
    /// renderer does not populate. Not advertised, not importable — yet.
    Chroma,
    /// No VkFormat describes this memory layout (or it is not a format at all).
    None(&'static str),
}

const ORDER: &str = "no VkFormat has this channel order";
const EXT: &str = "layout exists only via VK_EXT_4444_formats / VK_KHR_maintenance5";
const AUX: &str = "two-plane RGB + A8; Vulkan has no such layout";
const NOT_A_FORMAT: &str = "DRM_FORMAT_BIG_ENDIAN is a flag bit, not a format";
const INDEXED: &str = "paletted/indexed; no VkFormat and no palette plumbing";

const fn img(format: vk::Format) -> Vk {
    Vk::Image { format, opaque: false, linear: false }
}

/// An `X` (padding) format: same VkFormat as its `A` sibling, alpha forced to 1.
const fn opq(format: vk::Format) -> Vk {
    Vk::Image { format, opaque: true, linear: false }
}

/// A float format: linear-light / extended-range values, not sRGB-encoded.
const fn imgf(format: vk::Format) -> Vk {
    Vk::Image { format, opaque: false, linear: true }
}

/// Float and opaque (`X`).
const fn opqf(format: vk::Format) -> Vk {
    Vk::Image { format, opaque: true, linear: true }
}

macro_rules! fourcc_table {
    ($($code:ident => $mapping:expr),+ $(,)?) => {
        /// Every `Fourcc` variant, in drm-fourcc declaration order. Generated
        /// from the same list as [`classify`], so it cannot go stale.
        pub const ALL: &[Fourcc] = &[$(Fourcc::$code),+];

        /// The Vulkan answer for `fourcc`. NO WILDCARD ARM — see the module docs.
        pub fn classify(fourcc: Fourcc) -> Vk {
            match fourcc { $(Fourcc::$code => $mapping),+ }
        }
    };
}

fourcc_table! {
    Abgr1555 => Vk::None(EXT),                                  // A1B5G5R5 is maintenance5-only
    Abgr16161616f => imgf(vk::Format::R16G16B16A16_SFLOAT),
    Abgr2101010 => img(vk::Format::A2B10G10R10_UNORM_PACK32),
    Abgr4444 => Vk::None(EXT),
    Abgr8888 => img(vk::Format::R8G8B8A8_UNORM),
    Argb1555 => img(vk::Format::A1R5G5B5_UNORM_PACK16),
    Argb16161616f => Vk::None(ORDER),                           // no B16G16R16A16_SFLOAT
    Argb2101010 => img(vk::Format::A2R10G10B10_UNORM_PACK32),
    Argb4444 => Vk::None(EXT),
    Argb8888 => img(vk::Format::B8G8R8A8_UNORM),
    Axbxgxrx106106106106 => Vk::None(ORDER),
    Ayuv => Vk::Chroma,
    Bgr233 => Vk::None(ORDER),
    Bgr565 => img(vk::Format::B5G6R5_UNORM_PACK16),
    Bgr565_a8 => Vk::None(AUX),
    Bgr888 => img(vk::Format::R8G8B8_UNORM),                    // 24bpp, memory R,G,B
    Bgr888_a8 => Vk::None(AUX),
    Bgra1010102 => Vk::None(ORDER),
    Bgra4444 => img(vk::Format::B4G4R4A4_UNORM_PACK16),
    Bgra5551 => img(vk::Format::B5G5R5A1_UNORM_PACK16),
    Bgra8888 => Vk::None(ORDER),                                // memory A,R,G,B
    Bgrx1010102 => Vk::None(ORDER),
    Bgrx4444 => opq(vk::Format::B4G4R4A4_UNORM_PACK16),
    Bgrx5551 => opq(vk::Format::B5G5R5A1_UNORM_PACK16),
    Bgrx8888 => Vk::None(ORDER),
    Bgrx8888_a8 => Vk::None(AUX),
    Big_endian => Vk::None(NOT_A_FORMAT),
    C8 => Vk::None(INDEXED),
    Gr1616 => img(vk::Format::R16G16_UNORM),                    // memory R16,G16
    Gr88 => img(vk::Format::R8G8_UNORM),
    Nv12 => Vk::Chroma,
    Nv15 => Vk::Chroma,
    Nv16 => Vk::Chroma,
    Nv21 => Vk::Chroma,
    Nv24 => Vk::Chroma,
    Nv42 => Vk::Chroma,
    Nv61 => Vk::Chroma,
    P010 => Vk::Chroma,
    P012 => Vk::Chroma,
    P016 => Vk::Chroma,
    P210 => Vk::Chroma,
    Q401 => Vk::Chroma,
    Q410 => Vk::Chroma,
    R16 => img(vk::Format::R16_UNORM),
    R8 => img(vk::Format::R8_UNORM),
    Rg1616 => Vk::None(ORDER),                                  // would need G16R16
    Rg88 => Vk::None(ORDER),                                    // would need G8R8
    Rgb332 => Vk::None(ORDER),
    Rgb565 => img(vk::Format::R5G6B5_UNORM_PACK16),
    Rgb565_a8 => Vk::None(AUX),
    Rgb888 => img(vk::Format::B8G8R8_UNORM),                    // 24bpp, memory B,G,R
    Rgb888_a8 => Vk::None(AUX),
    Rgba1010102 => Vk::None(ORDER),
    Rgba4444 => img(vk::Format::R4G4B4A4_UNORM_PACK16),
    Rgba5551 => img(vk::Format::R5G5B5A1_UNORM_PACK16),
    Rgba8888 => Vk::None(ORDER),                                // memory A,B,G,R
    Rgbx1010102 => Vk::None(ORDER),
    Rgbx4444 => opq(vk::Format::R4G4B4A4_UNORM_PACK16),
    Rgbx5551 => opq(vk::Format::R5G5B5A1_UNORM_PACK16),
    Rgbx8888 => Vk::None(ORDER),
    Rgbx8888_a8 => Vk::None(AUX),
    Uyvy => Vk::Chroma,
    Vuy101010 => Vk::Chroma,
    Vuy888 => Vk::Chroma,
    Vyuy => Vk::Chroma,
    X0l0 => Vk::Chroma,
    X0l2 => Vk::Chroma,
    Xbgr1555 => Vk::None(EXT),
    Xbgr16161616f => opqf(vk::Format::R16G16B16A16_SFLOAT),      // XB4H
    Xbgr2101010 => opq(vk::Format::A2B10G10R10_UNORM_PACK32),
    Xbgr4444 => Vk::None(EXT),
    Xbgr8888 => opq(vk::Format::R8G8B8A8_UNORM),
    Xbgr8888_a8 => Vk::None(AUX),
    Xrgb1555 => opq(vk::Format::A1R5G5B5_UNORM_PACK16),
    Xrgb16161616f => Vk::None(ORDER),
    Xrgb2101010 => opq(vk::Format::A2R10G10B10_UNORM_PACK32),
    Xrgb4444 => Vk::None(EXT),
    Xrgb8888 => opq(vk::Format::B8G8R8A8_UNORM),
    Xrgb8888_a8 => Vk::None(AUX),
    Xvyu12_16161616 => Vk::Chroma,
    Xvyu16161616 => Vk::Chroma,
    Xvyu2101010 => Vk::Chroma,
    Xyuv8888 => Vk::Chroma,
    Y0l0 => Vk::Chroma,
    Y0l2 => Vk::Chroma,
    Y210 => Vk::Chroma,
    Y212 => Vk::Chroma,
    Y216 => Vk::Chroma,
    Y410 => Vk::Chroma,
    Y412 => Vk::Chroma,
    Y416 => Vk::Chroma,
    Yuv410 => Vk::Chroma,
    Yuv411 => Vk::Chroma,
    Yuv420 => Vk::Chroma,
    Yuv420_10bit => Vk::Chroma,
    Yuv420_8bit => Vk::Chroma,
    Yuv422 => Vk::Chroma,
    Yuv444 => Vk::Chroma,
    Yuyv => Vk::Chroma,
    Yvu410 => Vk::Chroma,
    Yvu411 => Vk::Chroma,
    Yvu420 => Vk::Chroma,
    Yvu422 => Vk::Chroma,
    Yvu444 => Vk::Chroma,
    Yvyu => Vk::Chroma,
}

/// Whether `fourcc`'s values are linear-light / extended-range rather than
/// sRGB-encoded — i.e. whether reading them needs a composite that knows the
/// difference. Only the float formats; `Vk::None`/`Chroma` are `false` because
/// they are refused for a stronger reason already.
pub fn extended_range(fourcc: Fourcc) -> bool {
    matches!(classify(fourcc), Vk::Image { linear: true, .. })
}

/// Every fourcc the Vulkan path can import as a colour image, with its VkFormat
/// and whether the view has to force alpha. THE source for both the advertised
/// set and the import decision — device filtering happens on top of this.
pub fn images() -> impl Iterator<Item = (Fourcc, vk::Format, bool)> {
    ALL.iter().filter_map(|&code| match classify(code) {
        Vk::Image { format, opaque, .. } => Some((code, format, opaque)),
        Vk::Chroma | Vk::None(_) => None,
    })
}
