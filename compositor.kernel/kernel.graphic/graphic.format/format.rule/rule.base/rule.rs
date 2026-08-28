//! The rules every format decision obeys, stated once.
//!
//! Modifier classification and ranking (moved verbatim from the former
//! `bridge.negotiate/negotiate.classify`), plus the single vocabulary for what an
//! empty answer MEANS.
//!
//! # Why the outcome vocabulary is here and not at the call sites
//!
//! Before this crate the tree had four mutually incompatible answers to "the
//! negotiation came back empty": the scanout narrowing aborted the process at
//! startup, the producer paths returned an empty list that turned fatal later at
//! the allocator, the client advertisement logged an error and advertised nothing
//! (silently dropping every client to shm), and the overview blur alone degraded
//! gracefully. Four call sites, four policies, no shared word for the condition —
//! so each was one edit from diverging further. [`Outcome`] is that word.

use smithay::backend::allocator::Modifier;

/// Coarse class of a DRM modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModClass {
    /// `DRM_FORMAT_MOD_LINEAR` (0).
    Linear,
    /// `DRM_FORMAT_MOD_INVALID` (implicit / driver-negotiated).
    ///
    /// NOT a member of the lattice: it names "ask the driver", so it cannot be
    /// meaningfully intersected with anything. It may be carried through where a
    /// set is unpublished, and must be dropped before a set reaches KMS.
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
///
/// KNOWN GAP, deliberately left as-is here: Intel CCS modifiers carry their
/// compression metadata in an extra plane rather than in a vendor bit, so they
/// classify as plain [`ModClass::Tiled`] and [`is_dcc`] misses them. `FORCE_MULTIPLANE`
/// therefore empties the candidate list on Intel. The real fix is to decide this
/// from the modifier's PLANE COUNT — which the device query already returns and
/// this layer currently discards — rather than from a vendor bit; that is a
/// behaviour change and belongs with the audit work, not with a move.
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

/// One modifier, as `class:hex` — `linear`, `invalid`, `tiled:0x300000000606014`.
///
/// The class alone is ambiguous (a device reports six distinct tiled modifiers)
/// and the raw u64 alone is unreadable, so every log that names a modifier names
/// both, the same way everywhere.
pub fn describe(m: Modifier) -> String {
    match classify(m) {
        ModClass::Linear => "linear".into(),
        ModClass::Invalid => "invalid".into(),
        c => format!("{}:{:#x}", label(c), u64::from(m)),
    }
}

/// A modifier list, best-first order preserved. `-` when empty, so an empty list
/// is visibly empty rather than a blank at the end of a line.
pub fn describe_all(mods: &[Modifier]) -> String {
    if mods.is_empty() {
        return "-".into();
    }
    mods.iter().map(|m| describe(*m)).collect::<Vec<_>>().join(" ")
}

/// "No modifier is known here." Used by diagnostic paths that must print or seed
/// a modifier before one has been chosen — NOT a claim that a buffer is implicit.
pub const UNKNOWN: Modifier = Modifier::Invalid;

/// Whether a modifier survives the Law-7 legacy filter: `LINEAR` and the
/// driver-negotiated implicit modifier only, the documented safe set for hardware
/// classes whose compression modifiers fail under load.
pub fn legacy(m: Modifier) -> bool {
    matches!(m, Modifier::Linear | Modifier::Invalid)
}

/// What an answer from this layer MEANS when it is not a plain success.
///
/// The distinction that matters is [`Self::Degraded`] versus [`Self::Refused`]:
/// degraded means a constraint was DROPPED because nothing had published it yet,
/// so the answer is wider than it should be and the caller may proceed; refused
/// means the constraints were all present and genuinely share nothing, so there
/// is no safe buffer to allocate and proceeding would produce the unimportable
/// modifier this layer exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every term was published and the result is non-empty.
    Ok,
    /// A term was unpublished and dropped. The answer is wider than the truth.
    Degraded(&'static str),
    /// Every term was present and they intersect in nothing.
    Refused(&'static str),
}

impl Outcome {
    /// Whether a caller may allocate on this answer.
    pub fn usable(self) -> bool {
        !matches!(self, Self::Refused(_))
    }
}
