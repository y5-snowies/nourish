//! Cross-API `(fourcc × modifier)` intersection for the dmabuf bridge, plus the
//! experimental-flag-driven modifier selection the bridge allocator honors.

use std::collections::HashSet;

use compositor_model_environment_experimental_base::base::GpuFlags;
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

    /// The gbm modifier list to allocate `fourcc` with under the experimental
    /// flags (see the plan for composition). Empty ⇒ implicit path.
    pub fn modifiers_for(&self, fourcc: Fourcc, flags: GpuFlags, raw: &[String]) -> Vec<Modifier> {
        let mut mods: Vec<Modifier> = self
            .set
            .iter()
            .filter(|f| f.code == fourcc)
            .map(|f| f.modifier)
            .collect();
        mods.sort_by_key(|m| std::cmp::Reverse(rank(*m))); // best-first

        let (force_linear, force_tiled) = resolve_force(flags, raw);
        // Negotiation is the default; only the opt-out flag falls back to implicit.
        // With it set (and no force flag) the `(false, false, false)` arm yields an
        // empty list → the allocator's byte-identical implicit path.
        let negotiate = !flags.contains(GpuFlags::NO_NEGOTIATE_MODIFIERS);

        let mut result = match (negotiate, force_linear, force_tiled) {
            (false, false, false) if !flags.contains(GpuFlags::FORCE_MULTIPLANE) => Vec::new(),
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
        if flags.contains(GpuFlags::FORCE_MULTIPLANE) {
            result.retain(|m| is_dcc(*m));
        }
        result
    }
}

/// Convenience for bridge call sites: intersect the renderer-importable and
/// wgpu-importable sets and resolve the modifier list for `fourcc` under the
/// current experimental flags. Empty result ⇒ the allocator's implicit path.
pub fn bridge_modifiers(
    renderer: FormatSet,
    wgpu_importable: FormatSet,
    fourcc: Fourcc,
) -> Vec<Modifier> {
    use compositor_model_environment_experimental_base::base as ex;
    let (had_renderer, had_wgpu) = (renderer.iter().count(), wgpu_importable.iter().count());
    let mods = BridgeFormats::intersect(&[renderer, wgpu_importable])
        .modifiers_for(fourcc, ex::get(), ex::raw());
    report(fourcc, had_renderer, had_wgpu, &mods);
    mods
}

/// Say so, ONCE, when two non-empty format sets share nothing.
///
/// This is the split-device signature and it is otherwise completely silent: an
/// empty list is the allocator's documented "use the implicit path", which is
/// also what a single-GPU machine with the negotiation flag off produces. The
/// difference is that here it was not asked for — the two ends are on different
/// GPUs and their vendor tiled modifiers (`I915_FORMAT_MOD_*` against
/// `AMD_FMT_MOD_*` or NVIDIA block-linear) simply share no values, so the exact
/// `(fourcc, modifier)` intersection empties out. The implicit allocation that
/// follows can report modifier INVALID, which is the wgpu-import failure the
/// negotiated path exists to prevent.
///
/// Latched, because this is a property of the machine's configuration, not of
/// the call — and the call happens per slot per surface per resize.
fn report(fourcc: Fourcc, renderer: usize, wgpu: usize, mods: &[Modifier]) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SAID: AtomicBool = AtomicBool::new(false);
    if !mods.is_empty() || renderer == 0 || wgpu == 0 {
        return;
    }
    if SAID.swap(true, Ordering::Relaxed) {
        return;
    }
    warn!(
        "bridge: no shared modifier for {fourcc:?} — the renderer offers {renderer} format(s) \
         and wgpu {wgpu}, and they intersect in none. Falling back to implicit allocation, \
         which can yield modifier INVALID and be refused on import. This is what a \
         render_node on a different GPU from the scanout device looks like; check \
         `gpu topology:` above."
    );
}

/// Resolve FORCE_LINEAR / FORCE_TILED; when both are set, last-in-`raw` wins.
fn resolve_force(flags: GpuFlags, raw: &[String]) -> (bool, bool) {
    let fl = flags.contains(GpuFlags::FORCE_LINEAR);
    let ft = flags.contains(GpuFlags::FORCE_TILED);
    if fl && ft {
        match raw
            .iter()
            .rev()
            .find(|s| *s == "gpu_force_linear" || *s == "gpu_force_tiled")
            .map(String::as_str)
        {
            Some("gpu_force_tiled") => (false, true),
            _ => (true, false),
        }
    } else {
        (fl, ft)
    }
}
