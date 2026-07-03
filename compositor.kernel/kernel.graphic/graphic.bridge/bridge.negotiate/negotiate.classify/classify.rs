//! Classify a DRM format modifier — used for selection ranking and the
//! developer-tool "GPU formats" labels. Pure functions of the modifier value.

use smithay::backend::allocator::Modifier;

/// Coarse class of a DRM modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModClass {
    /// `DRM_FORMAT_MOD_LINEAR` (0).
    Linear,
    /// `DRM_FORMAT_MOD_INVALID` (implicit / driver-negotiated).
    Invalid,
    /// A vendor tiled modifier, no compression metadata.
    Tiled,
    /// A tiled modifier carrying compression metadata (AMD DCC — multi-plane).
    TiledDcc,
}

const VENDOR_AMD: u64 = 0x02; // DRM_FORMAT_MOD_VENDOR_AMD
const AMD_FMT_MOD_DCC_SHIFT: u64 = 13;

fn vendor(m: u64) -> u64 {
    (m >> 56) & 0xff
}

/// Classify a modifier. DCC detection covers the AMD_FMT_MOD encoding.
pub fn classify(m: Modifier) -> ModClass {
    match m {
        Modifier::Linear => ModClass::Linear,
        Modifier::Invalid => ModClass::Invalid,
        _ => {
            let v: u64 = m.into();
            if vendor(v) == VENDOR_AMD && (v >> AMD_FMT_MOD_DCC_SHIFT) & 1 == 1 {
                ModClass::TiledDcc
            } else {
                ModClass::Tiled
            }
        }
    }
}

/// A tiled (non-linear, non-invalid) modifier.
pub fn is_tiled(m: Modifier) -> bool {
    matches!(classify(m), ModClass::Tiled | ModClass::TiledDcc)
}

/// A compression-metadata (DCC — typically multi-plane) modifier.
pub fn is_dcc(m: Modifier) -> bool {
    matches!(classify(m), ModClass::TiledDcc)
}

/// Selection rank, best first: tiled > linear > invalid.
pub fn rank(m: Modifier) -> u8 {
    match classify(m) {
        ModClass::Tiled | ModClass::TiledDcc => 3,
        ModClass::Linear => 2,
        ModClass::Invalid => 1,
    }
}

/// Short human label for the developer tool.
pub fn label(c: ModClass) -> &'static str {
    match c {
        ModClass::Linear => "linear",
        ModClass::Invalid => "invalid",
        ModClass::Tiled => "tiled",
        ModClass::TiledDcc => "tiled+dcc",
    }
}
