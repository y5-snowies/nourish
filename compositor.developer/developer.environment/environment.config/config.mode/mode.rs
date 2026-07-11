//! The per-render-node `mode` token set and its folded [`ModeFlags`].
//!
//! Tokens MIRROR stable's `gpu_*` experimental flags exactly — same names, same
//! polarity (the opt-OUT flags stay opt-OUT) — plus new topology/routing/perf
//! tokens. An empty token list folds to `ModeFlags::empty()`, byte-identical to
//! stable HEAD with an empty `experimental.json`. See `document/GPU_TOPOLOGY.md`.

use bitflags::bitflags;

/// One `mode` token, authored as a JSON string. Unknown tokens are a hard serde
/// parse error at load (strict). The list COMPOSES.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeToken {
    /// Session anchor for singular roles. Implicit for a single-entry map.
    SessionPrimary,
    /// A secondary render/offload node (import source; not compositing unless `LocalRender`).
    Render,
    /// Opt OUT of modifier negotiation (absent = negotiate, stable default).
    NoNegotiateModifiers,
    ForceLinear,
    ForceTiled,
    AllowDcc,
    /// Retain only DCC modifiers; implies `AllowDcc`.
    ForceMultiplane,
    ProbeModifiers,
    /// Opt OUT of pinning wgpu to this node (absent = pin, stable default).
    NoPinWgpu,
    /// Opt OUT of overlay/cursor direct scanout (absent = allow, stable default).
    NoDirectScanout,
    ScanoutBridge,
    UntileBlit,
    DmabufMainDevice,
    ZeroCopy,
    LocalRender,
    RenderDiscovery,
    ScanoutDiscovery,
    BlitFallback,
}

bitflags! {
    /// Folded `mode` bits. Polarity mirrors stable: `NO_*` bits mean "opt out",
    /// so an empty set == stable defaults (consumers keep `!contains(NO_*)`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct ModeFlags: u32 {
        const SESSION_PRIMARY = 1 << 0;
        const RENDER = 1 << 1;
        const NO_NEGOTIATE_MODIFIERS = 1 << 2;
        const FORCE_LINEAR = 1 << 3;
        const FORCE_TILED = 1 << 4;
        const ALLOW_DCC = 1 << 5;
        const FORCE_MULTIPLANE = 1 << 6;
        const PROBE_MODIFIERS = 1 << 7;
        const NO_PIN_WGPU = 1 << 8;
        const NO_DIRECT_SCANOUT = 1 << 9;
        const SCANOUT_BRIDGE = 1 << 10;
        const UNTILE_BLIT = 1 << 11;
        const DMABUF_MAIN_DEVICE = 1 << 12;
        const ZERO_COPY = 1 << 13;
        const LOCAL_RENDER = 1 << 14;
        const RENDER_DISCOVERY = 1 << 15;
        const SCANOUT_DISCOVERY = 1 << 16;
        const BLIT_FALLBACK = 1 << 17;
    }
}

impl ModeToken {
    /// The single bit this token sets.
    pub fn flag(self) -> ModeFlags {
        match self {
            ModeToken::SessionPrimary => ModeFlags::SESSION_PRIMARY,
            ModeToken::Render => ModeFlags::RENDER,
            ModeToken::NoNegotiateModifiers => ModeFlags::NO_NEGOTIATE_MODIFIERS,
            ModeToken::ForceLinear => ModeFlags::FORCE_LINEAR,
            ModeToken::ForceTiled => ModeFlags::FORCE_TILED,
            ModeToken::AllowDcc => ModeFlags::ALLOW_DCC,
            ModeToken::ForceMultiplane => ModeFlags::FORCE_MULTIPLANE,
            ModeToken::ProbeModifiers => ModeFlags::PROBE_MODIFIERS,
            ModeToken::NoPinWgpu => ModeFlags::NO_PIN_WGPU,
            ModeToken::NoDirectScanout => ModeFlags::NO_DIRECT_SCANOUT,
            ModeToken::ScanoutBridge => ModeFlags::SCANOUT_BRIDGE,
            ModeToken::UntileBlit => ModeFlags::UNTILE_BLIT,
            ModeToken::DmabufMainDevice => ModeFlags::DMABUF_MAIN_DEVICE,
            ModeToken::ZeroCopy => ModeFlags::ZERO_COPY,
            ModeToken::LocalRender => ModeFlags::LOCAL_RENDER,
            ModeToken::RenderDiscovery => ModeFlags::RENDER_DISCOVERY,
            ModeToken::ScanoutDiscovery => ModeFlags::SCANOUT_DISCOVERY,
            ModeToken::BlitFallback => ModeFlags::BLIT_FALLBACK,
        }
    }
}

/// Fold an ordered token list into [`ModeFlags`], preserving stable semantics:
/// the `force_linear`/`force_tiled` conflict is resolved **last-wins by order**,
/// and `force_multiplane` implies `allow_dcc`.
pub fn fold(tokens: &[ModeToken]) -> ModeFlags {
    let mut flags = ModeFlags::empty();
    let mut last_force: Option<ModeFlags> = None;
    for t in tokens {
        let bit = t.flag();
        if bit == ModeFlags::FORCE_LINEAR || bit == ModeFlags::FORCE_TILED {
            last_force = Some(bit);
        }
        flags |= bit;
    }
    if flags.contains(ModeFlags::FORCE_LINEAR) && flags.contains(ModeFlags::FORCE_TILED) {
        flags.remove(ModeFlags::FORCE_LINEAR | ModeFlags::FORCE_TILED);
        flags |= last_force.expect("both force bits set implies a force token was seen");
    }
    if flags.contains(ModeFlags::FORCE_MULTIPLANE) {
        flags |= ModeFlags::ALLOW_DCC;
    }
    flags
}
