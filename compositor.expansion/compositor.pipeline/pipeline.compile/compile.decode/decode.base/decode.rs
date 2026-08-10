//! Decode a bundle's declared textures to RGBA8, under caps that are checked
//! BEFORE anything is allocated.
//!
//! # Why the caps come first
//!
//! A bundle is a folder a user dropped in, so its files are untrusted input in
//! the ordinary sense: a 16384×16384 PNG is a few hundred kilobytes on disk and
//! a gigabyte decoded, and the allocation that kills the compositor happens
//! inside the decoder, before any of our code runs again. So the dimensions are
//! read from the file HEADER — which costs a few bytes — and refused there.
//!
//! The same reasoning gives the budget its shape: it is counted on DECODED bytes
//! (`w * h * 4`), never on file size. File size is the wrong number by two or
//! three orders of magnitude and in the attacker-chosen direction.
//!
//! Refused, never trimmed — same rule the storage budget follows. A bundle handed
//! a smaller image than it asked for samples the wrong thing everywhere and has
//! no way to find out.

use std::sync::Arc;

/// The longest edge any single texture may have.
///
/// 4096 is past every plausible sprite sheet and LUT strip while keeping the
/// worst case decode near 67 MB and well inside a fraction of a second — which
/// matters because decoding happens on the thread that selected the bundle.
pub const MAX_EDGE: u32 = 4096;

/// The most one bundle may hold in textures, decoded.
///
/// Deliberately the same figure as `STORAGE_BUDGET`: both are author-chosen
/// magnitudes that land in device memory and both are refused rather than
/// trimmed, so giving them one number to remember is worth more than tuning
/// each.
pub const TEXTURE_BUDGET: u64 = 64 << 20;

/// One decoded texture: tightly packed RGBA8, top row first.
#[derive(Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Whether the samples are sRGB-encoded, carried through from the manifest so
    /// the renderer can pick `R8G8B8A8_SRGB` over `_UNORM` and let the hardware
    /// linearise. See `manifest::Texture::srgb`.
    pub srgb: bool,
    /// `Arc` because this crosses the dispatch seam every frame while being read
    /// only on the first — the same reason a pass's SPIR-V is one.
    pub pixels: Arc<[u8]>,
}

impl Decoded {
    /// Decoded size in device memory.
    pub fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
}

/// Decode `bytes` (a PNG or JPEG) for the texture declared as `name`.
///
/// `name` rather than the file path in the messages: it is what the manifest and
/// the failing pass's `inputs` both say, so an author reads the error in the
/// vocabulary they wrote.
pub fn decode(name: &str, bytes: &[u8], srgb: bool) -> Result<Decoded, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("texture '{name}': {e}"))?;
    let format = reader
        .format()
        .ok_or_else(|| format!("texture '{name}': not a PNG or JPEG (unrecognised header)"))?;
    // Dimensions off the header, before the decoder allocates anything.
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("texture '{name}': unreadable header: {e}"))?;
    if w == 0 || h == 0 {
        return Err(format!("texture '{name}': {w}x{h} has no pixels"));
    }
    if w > MAX_EDGE || h > MAX_EDGE {
        return Err(format!(
            "texture '{name}': {w}x{h} exceeds the {MAX_EDGE}x{MAX_EDGE} limit — refused before \
             decoding, because the allocation that would fail happens inside the decoder"
        ));
    }
    let img = image::load_from_memory_with_format(bytes, format)
        .map_err(|e| format!("texture '{name}': {e}"))?
        .into_rgba8();
    Ok(Decoded {
        width: img.width(),
        height: img.height(),
        srgb,
        // Straight alpha, exactly as authored — see `manifest::Texture`.
        pixels: Arc::from(img.into_raw().into_boxed_slice()),
    })
}

/// Refuse a set of textures whose decoded total is past the budget.
///
/// Checked over the whole set, not per file, so twenty medium images cannot slip
/// past a per-image limit the way twenty small storage buffers could not.
pub fn budget(list: &[Decoded]) -> Result<(), String> {
    let asked: u64 = list.iter().map(Decoded::bytes).sum();
    if asked > TEXTURE_BUDGET {
        return Err(format!(
            "textures: this bundle decodes to {asked} bytes across {} images; the budget is \
             {TEXTURE_BUDGET}. Refused rather than trimmed — a bundle sampling an image smaller \
             than the one it declared reads the wrong pixels everywhere and cannot tell.",
            list.len(),
        ));
    }
    Ok(())
}

/// Whether `rel` is a path a texture may name.
///
/// Textures are restricted to the bundle directory, which `modules` deliberately
/// are not — sibling bundles share WGSL through `../other-bundle/lib/x.wgsl` and
/// that is a feature. A texture has no such use, and an unrestricted path is a
/// bundle that can read any file the compositor can and paint it on the desktop.
pub fn contained(rel: &str) -> bool {
    let p = std::path::Path::new(rel);
    !p.is_absolute()
        && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))
        && !rel.is_empty()
}
