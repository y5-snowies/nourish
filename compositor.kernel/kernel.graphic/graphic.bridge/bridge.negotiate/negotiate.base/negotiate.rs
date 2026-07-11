//! Cross-API `(fourcc × modifier)` intersection for the dmabuf bridge, plus the
//! per-node `mode`-driven modifier selection the bridge allocator honors.

use std::collections::HashSet;

use compositor_developer_environment_config_mode::mode::ModeFlags;
use compositor_kernel_graphic_bridge_negotiate_classify::classify::{is_dcc, is_tiled, rank};
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};

/// The `(fourcc, modifier)` pairs importable by EVERY bridge participant
/// (gbm ∩ renderer ∩ wgpu). An empty [`modifiers_for`] result means "use the
/// implicit, byte-identical allocation path".
#[derive(Debug, Clone, Default)]
pub struct BridgeFormats {
    pub set: FormatSet,
}

impl BridgeFormats {
    /// Set-intersect several backends' format sets by exact `(fourcc, modifier)`.
    /// No sources → empty.
    pub fn intersect(sources: &[FormatSet]) -> BridgeFormats {
        let mut it = sources.iter();
        let Some(first) = it.next() else {
            return BridgeFormats::default();
        };
        let mut acc: HashSet<DrmFormat> = first.iter().copied().collect();
        for s in it {
            let cur: HashSet<DrmFormat> = s.iter().copied().collect();
            acc.retain(|f| cur.contains(f));
        }
        BridgeFormats {
            set: acc.into_iter().collect(),
        }
    }

    /// The fourccs surviving the intersection (color-format intersection).
    pub fn fourccs(&self) -> Vec<Fourcc> {
        let mut v: Vec<Fourcc> = self.set.iter().map(|f| f.code).collect();
        v.sort_by_key(|c| *c as u32);
        v.dedup();
        v
    }

    /// The gbm modifier list to allocate `fourcc` with under the render node's
    /// `mode` (polarity preserved from stable). Empty ⇒ implicit path.
    pub fn modifiers_for(&self, fourcc: Fourcc, mode: ModeFlags) -> Vec<Modifier> {
        let mut mods: Vec<Modifier> = self
            .set
            .iter()
            .filter(|f| f.code == fourcc)
            .map(|f| f.modifier)
            .collect();
        mods.sort_by_key(|m| std::cmp::Reverse(rank(*m))); // best-first

        // Negotiation is the default; only the opt-out `no_negotiate_modifiers`
        // token falls back to implicit. With it set (and no force token) the
        // `(false, false, false)` arm yields an empty list → the allocator's
        // byte-identical implicit path. FORCE_LINEAR/FORCE_TILED are already
        // mutually exclusive (fold resolves the last-wins conflict).
        let negotiate = !mode.contains(ModeFlags::NO_NEGOTIATE_MODIFIERS);
        let force_linear = mode.contains(ModeFlags::FORCE_LINEAR);
        let force_tiled = mode.contains(ModeFlags::FORCE_TILED);

        let mut result = match (negotiate, force_linear, force_tiled) {
            (false, false, false) if !mode.contains(ModeFlags::FORCE_MULTIPLANE) => Vec::new(),
            (false, true, _) => vec![Modifier::Linear],
            (false, false, true) => mods.into_iter().filter(|m| is_tiled(*m)).collect(),
            (true, false, false) | (false, false, false) => mods,
            (true, true, _) => {
                mods.sort_by_key(|m| *m != Modifier::Linear); // bias linear (stable)
                mods
            }
            (true, false, true) => {
                mods.sort_by_key(|m| !is_tiled(*m)); // bias tiled (stable)
                mods
            }
        };
        // Require a multi-plane (DCC) modifier — drop everything else. If none
        // survive, the empty list falls back to the implicit path at the allocator.
        if mode.contains(ModeFlags::FORCE_MULTIPLANE) {
            result.retain(|m| is_dcc(*m));
        }
        result
    }
}

/// Convenience for bridge call sites: intersect the renderer-importable and
/// wgpu-importable sets and resolve the modifier list for `fourcc` under the
/// render node's `mode`. Empty result ⇒ the allocator's implicit path.
pub fn bridge_modifiers(
    renderer: FormatSet,
    wgpu_importable: FormatSet,
    fourcc: Fourcc,
    mode: ModeFlags,
) -> Vec<Modifier> {
    BridgeFormats::intersect(&[renderer, wgpu_importable]).modifiers_for(fourcc, mode)
}

/// Whether the render-importable ∩ wgpu-importable intersection has NO modifier
/// for `fourcc` — the empty-intersection case that mandates the untiling blit
/// floor on a split render/scanout system (see `document/GPU_UNTILE_BLIT.md`).
///
/// Unlike [`bridge_modifiers`], this ignores the FORCE_*/NO_NEGOTIATE flags: it
/// answers the pure hardware question "can any single buffer satisfy both sides?"
/// so the two-buffer decision doesn't hinge on a modifier-selection preference.
pub fn bridge_intersection_empty(
    renderer: FormatSet,
    wgpu_importable: FormatSet,
    fourcc: Fourcc,
) -> bool {
    let shared = BridgeFormats::intersect(&[renderer, wgpu_importable]);
    !shared.set.iter().any(|f| f.code == fourcc)
}
