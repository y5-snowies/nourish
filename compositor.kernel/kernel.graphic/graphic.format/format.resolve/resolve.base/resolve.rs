//! The ONLY place `(fourcc, modifier)` sets are combined.
//!
//! Everything here was previously spread across five call sites that did not agree
//! with each other: a generic N-way intersector, a 2-way bridge convenience, a
//! 3-way inline-surface variant, the scanout narrowing (which aborted on empty)
//! and the client advertisement (which is deliberately not an intersection at
//! all). They disagreed on what an empty result means, on whether `INVALID`
//! survives, and on how `LINEAR` is ranked — and two of them ran on the same value
//! in the same expression, one adding `INVALID` and the next removing it.
//!
//! Concentrating them does not make the rules simpler; it makes them ONE rule that
//! can be read in one sitting, which is the property that was missing.

use compositor_kernel_graphic_format_audit_base::audit;
use compositor_kernel_graphic_format_registrar_base::registrar::{Registrar, View};
use compositor_kernel_graphic_format_role_base::role::Role;
use compositor_kernel_graphic_format_rule_base::rule::{self, Outcome};
use compositor_kernel_graphic_format_catalog_base::catalog;
use compositor_model_environment_experimental_base::base::GpuFlags;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};
use std::collections::HashSet;

/// Set-intersect several capability sets by exact `(fourcc, modifier)`.
///
/// Exact pairing is the whole point: a fourcc that both sides know, through
/// modifiers neither shares, is a buffer neither can read. No sources → empty.
pub fn intersect(sources: &[FormatSet]) -> FormatSet {
    let mut it = sources.iter();
    let Some(first) = it.next() else {
        return FormatSet::default();
    };
    let mut acc: HashSet<DrmFormat> = first.iter().copied().collect();
    for s in it {
        let cur: HashSet<DrmFormat> = s.iter().copied().collect();
        acc.retain(|f| cur.contains(f));
    }
    acc.into_iter().collect()
}

/// The modifier list to allocate `fourcc` with, from an already-intersected set,
/// under the experimental flags.
///
/// Empty is a REFUSAL. It used to mean "use the implicit allocation path", but
/// that path is gone — it let the driver pick a modifier the compositor could not
/// import — and the allocator now ends the process rather than allocate without a
/// negotiated list.
pub fn modifiers_for(set: &FormatSet, fourcc: Fourcc, flags: GpuFlags, raw: &[String]) -> Vec<Modifier> {
    let mut mods: Vec<Modifier> = set
        .iter()
        .filter(|f| f.code == fourcc)
        .map(|f| f.modifier)
        .collect();
    mods.sort_by_key(|m| std::cmp::Reverse(rule::rank(*m))); // best-first

    let (force_linear, force_tiled) = resolve_force(flags, raw);
    // Negotiation is the default; only the opt-out flag declines it. With it set
    // (and no force flag) the `(false, false, false)` arm yields an empty list —
    // which the allocator reads as "refuse", so `gpu_no_negotiate_modifiers` stops
    // the compositor rather than loosening it.
    let negotiate = !flags.contains(GpuFlags::NO_NEGOTIATE_MODIFIERS);

    let mut result = match (negotiate, force_linear, force_tiled) {
        (false, false, false) if !flags.contains(GpuFlags::FORCE_MULTIPLANE) => Vec::new(),
        (false, true, _) => vec![Modifier::Linear],
        (false, false, true) => mods.into_iter().filter(|m| rule::is_tiled(*m)).collect(),
        (true, false, false) | (false, false, false) => mods,
        (true, true, _) => {
            mods.sort_by_key(|m| *m != Modifier::Linear); // bias linear (stable)
            mods
        }
        (true, false, true) => {
            mods.sort_by_key(|m| !rule::is_tiled(*m)); // bias tiled (stable)
            mods
        }
    };
    // Require a multi-plane (DCC) modifier — drop everything else. If none survive,
    // the empty list is a refusal and the allocator stops the compositor.
    if flags.contains(GpuFlags::FORCE_MULTIPLANE) {
        result.retain(|m| rule::is_dcc(*m));
    }
    result
}

/// Intersect two capability sets and resolve the modifier list for `fourcc`.
/// Empty ⇒ nothing can be allocated for `fourcc`.
pub fn bridge(renderer: FormatSet, wgpu: FormatSet, fourcc: Fourcc, what: &str) -> Vec<Modifier> {
    use compositor_model_environment_experimental_base::base as ex;
    let (had_renderer, had_wgpu) = (renderer.iter().count(), wgpu.iter().count());
    let joint = intersect(&[renderer, wgpu]);
    let mods = modifiers_for(&joint, fourcc, ex::get(), ex::raw());
    if mods.is_empty() {
        audit::outcome(
            what,
            if had_renderer == 0 || had_wgpu == 0 {
                Outcome::Degraded("one side enumerated no dmabuf formats at all")
            } else {
                Outcome::Refused("the renderer and wgpu sets intersect in no modifier")
            },
        );
        report_empty(fourcc, had_renderer, had_wgpu);
    }
    mods
}

/// Say so, ONCE, when the negotiation comes back empty — and say WHICH way.
///
/// A disjoint intersection is the split-device signature: the two ends are on
/// different GPUs and their vendor tiled modifiers share no values. An empty INPUT
/// is a different fault entirely — one side enumerated nothing — and telling them
/// apart is the whole job, because the allocation that follows is fatal either way
/// and the two have nothing in common as fixes.
fn report_empty(fourcc: Fourcc, renderer: usize, wgpu: usize) {
    use std::sync::atomic::{AtomicBool, Ordering};
    if renderer == 0 || wgpu == 0 {
        static SAID_EMPTY: AtomicBool = AtomicBool::new(false);
        if SAID_EMPTY.swap(true, Ordering::Relaxed) {
            return;
        }
        warn!(
            "bridge: nothing to negotiate for {fourcc:?} — the renderer offers {renderer} \
             format(s) and wgpu {wgpu}. An empty set is not a narrow one: whichever side \
             reports zero enumerated no dmabuf formats at all, so there is no modifier to \
             agree on and the allocation that follows cannot proceed."
        );
        return;
    }
    static SAID: AtomicBool = AtomicBool::new(false);
    if SAID.swap(true, Ordering::Relaxed) {
        return;
    }
    warn!(
        "bridge: no shared modifier for {fourcc:?} — the renderer offers {renderer} format(s) \
         and wgpu {wgpu}, and they intersect in none. There is nothing safe to allocate, so \
         the allocation that follows will stop the compositor. This is what a render_node on \
         a different GPU from the scanout device looks like; check `gpu topology:` above."
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

/// A producer's modifier list.
///
/// `renderer_terms` are intersected to form the "renderer" side; `other` is the
/// second side. Which roles those are differs per consumer and is NOT a detail —
/// the inline paths need GLES importability because they build a `GlesTexture` per
/// slot, the workers do not, and the overview blur pairs the composite against
/// GLES with no wgpu term at all because GLES is what writes its buffers.
///
/// An unpublished term is DROPPED, never intersected against. Intersecting an
/// empty set refuses everything, and an empty result here is fatal at the
/// allocator — on winit, where nothing published, that once meant every iced and
/// bevy surface failed to allocate and the whole UI went missing while the world
/// still drew.
pub fn producer_modifiers(
    r: &Registrar,
    v: &View,
    fourcc: Fourcc,
    renderer_terms: &[Role],
    other: Role,
    what: &str,
) -> Vec<Modifier> {
    // Producers are held to the same colour policy as clients. Without this a
    // worker could negotiate a format the composite cannot express — nobody
    // advertised it to them, they simply asked — and the renderer would refuse
    // its buffers at draw.
    if !catalog::expressible(fourcc, r.color_managed()) {
        return Vec::new();
    }
    let second = v.set_or_empty(other);
    // Per-term candidates FOR THIS FOURCC, captured before any narrowing so the
    // log can show what each side offered and exactly what survived.
    let mods_of = |s: &FormatSet| {
        let mut m: Vec<Modifier> = s.iter().filter(|f| f.code == fourcc).map(|f| f.modifier).collect();
        m.sort_by_key(|x| std::cmp::Reverse(rule::rank(*x)));
        m.dedup();
        m
    };
    let mut offered: Vec<(&'static str, Vec<Modifier>)> = renderer_terms
        .iter()
        .map(|r| (r.label(), mods_of(&v.set_or_empty(*r))))
        .collect();
    offered.push((other.label(), mods_of(&second)));

    let live: Vec<FormatSet> = renderer_terms
        .iter()
        .map(|r| v.set_or_empty(*r))
        .filter(|s| !s.indexset().is_empty())
        .collect();
    // Nothing on the renderer side registered: fall back to the second term alone,
    // which is what the previous `worker_modifiers` expressed by intersecting the
    // wgpu set with itself.
    let renderer = if live.is_empty() { second.clone() } else { intersect(&live) };
    let mods = bridge(renderer, second, fourcc, what);

    // LATCHED PER DISTINCT ANSWER. This block used to say "construction-time only
    // — every caller is allocating a ring slot, a capture entry or a swapchain,
    // never a frame", and that was wrong twice over: `catalog::wgpu_format`'s
    // caller asks for a producer's fourcc on EVERY dmabuf import (twice, once per
    // texture descriptor), and a surface whose backing is reallocated per frame
    // asks per frame. The result was five unlatched `info!` lines per allocation,
    // which is how a resize storm reads as a format-negotiation storm and buries
    // the thing that actually went wrong.
    //
    // The pairs are still printed — a count cannot answer the question these logs
    // exist for, "is the modifier I need actually in there" — just once per
    // distinct outcome. A repeat carries no information: the terms are properties
    // of the machine, so the same question has the same answer every time.
    //
    // `DrmModifier` is not `Ord`, so dedupe through the u64 it wraps.
    let mut dropped_raw: Vec<u64> = offered
        .iter()
        .flat_map(|(_, m)| m.iter().copied())
        .filter(|m| !mods.contains(m))
        .map(u64::from)
        .collect();
    dropped_raw.sort_unstable();
    dropped_raw.dedup();
    let dropped: Vec<Modifier> = dropped_raw.into_iter().map(Modifier::from).collect();
    if audit::first(format!("producer:{what}:{fourcc:?}:{}:{}", mods.len(), dropped.len())) {
        info!("format available: {what} -> {fourcc:?}");
        for (term, m) in &offered {
            info!("    offered by {term:<17} ({}): {}", m.len(), rule::describe_all(m));
        }
        info!("    GIVEN   ({}): {}", mods.len(), rule::describe_all(&mods));
        info!("    DROPPED ({}): {}", dropped.len(), rule::describe_all(&dropped));
    }
    mods
}

/// The set to ADVERTISE to clients: what the ACTIVE COMPOSITE can import, colour
/// policy applied. `egl` is NOT intersected in — it is the fallback source and the
/// log's baseline.
///
/// A client buffer is allocated by the client on `main_device` and sampled by the
/// composite. `main_device` IS the composite node, and this set is computed from
/// the composite's own physical device, so it answers both halves by itself. The
/// scanout device never touches a client buffer.
///
/// Keeping `egl` as an intersection term once the composite could move to another
/// device meant filtering an NVIDIA-allocated, NVIDIA-sampled buffer by what
/// Intel's EGL happens to enumerate — which on a cross-vendor split left exactly
/// one modifier per fourcc (LINEAR) and taxed every client. It never admitted a
/// wrong pair, only withheld right ones. Scanout-capability belongs in a feedback
/// TRANCHE, not in this set.
pub fn advertise(r: &Registrar, v: &View, egl: FormatSet) -> FormatSet {
    let importable = v.set_or_empty(Role::Sample);
    if importable.indexset().is_empty() {
        audit::outcome("advertise", Outcome::Degraded("no importable set published yet"));
        info!(
            "dmabuf feedback: no importable set published yet — advertising the EGL set \
             ({} pair(s)). Both backends now name `Sample` in their `Registrar::expect` \
             manifest and the advertisement is built by the loader once the registrar is \
             complete, so reaching this means the manifest and the registrations disagree.",
            egl.iter().count()
        );
        return egl.iter().filter(|f| catalog::expressible(f.code, r.color_managed())).copied().collect();
    }
    let offered: FormatSet =
        importable.iter().filter(|f| catalog::expressible(f.code, r.color_managed())).copied().collect();
    if offered.indexset().is_empty() {
        error!(
            "dmabuf feedback: the composite publishes {} importable pair(s) but the colour \
             policy withholds every one — advertising NOTHING, so clients fall back to shm.",
            importable.iter().count()
        );
        return offered;
    }
    audit::narrowing("advertise", &importable, &offered);
    // The pairs the composite offers that EGL never enumerated. Non-zero here IS
    // the widening: on a split device this is where the tiled modifiers come back.
    let beyond_egl = offered.iter().filter(|f| !egl.indexset().contains(*f)).count();
    if beyond_egl > 0 {
        info!(
            "dmabuf feedback: {beyond_egl} of {} advertised pair(s) are ones the scanout \
             device's EGL does not enumerate — offered because the COMPOSITE can sample them, \
             which is the only party that touches a client buffer",
            offered.iter().count()
        );
    }
    offered
}

/// The scanout swapchain's candidate set: the scanout device's EGL render set
/// narrowed to what the COMPOSITE can colour-attach, with `INVALID` dropped.
///
/// `INVALID` is stripped here rather than inherited: it means "ask the driver",
/// which is not a claim any device made, and handing it to KMS is how an
/// unimportable layout reaches the swapchain.
///
/// Returns `(set, outcome)`. A `Refused` outcome must end the process at startup —
/// proceeding produces a swapchain the composite cannot render into, every frame,
/// forever.
pub fn scanout(v: &View, egl: FormatSet, split_device: bool) -> (FormatSet, Outcome) {
    let renderable = v.set_or_empty(Role::Render);
    if renderable.indexset().is_empty() {
        return (egl, Outcome::Degraded("no composite-renderable set published"));
    }
    let narrowed: FormatSet = egl
        .intersection(&renderable)
        .filter(|f| f.modifier != Modifier::Invalid)
        .copied()
        .collect();
    audit::narrowing("scanout", &egl, &narrowed);
    if narrowed.indexset().is_empty() {
        let why = if split_device {
            "the scanout device's EGL set and the composite's renderable set share nothing \
             (a cross-vendor split: their vendor tiled modifiers have no values in common)"
        } else {
            "the scanout EGL set and the composite renderable set share nothing on one device"
        };
        return (narrowed, Outcome::Refused(why));
    }
    (narrowed, Outcome::Ok)
}

/// Whether the ACTIVE compositing renderer can import this exact pair.
///
/// `true` when nothing has published — the same degradation as everywhere else in
/// this layer, and for the same reason: refusing everything is worse than
/// answering wide.
pub fn importable_pair(r: &Registrar, fourcc: Fourcc, modifier: Modifier) -> bool {
    if !catalog::expressible(fourcc, r.color_managed()) {
        return false;
    }
    let importable = r.view().set_or_empty(Role::Sample);
    importable.indexset().is_empty()
        || importable.iter().any(|f| f.code == fourcc && f.modifier == modifier)
}

/// Whether the ACTIVE compositing renderer can import `fourcc` AT ALL —
/// fourcc-level and modifier-agnostic ON PURPOSE, since a v3/wl_drm client sends
/// `INVALID` and would fail an exact `(code, modifier)` test it should pass.
///
/// Used to refuse a buffer at CREATION instead of at draw: a client that gets
/// `zwp_linux_buffer_params.failed` can fall back, one whose window is blank
/// cannot.
pub fn importable_code(r: &Registrar, fourcc: Fourcc) -> bool {
    if !catalog::expressible(fourcc, r.color_managed()) {
        return false;
    }
    let importable = r.view().set_or_empty(Role::Sample);
    importable.indexset().is_empty() || importable.iter().any(|f| f.code == fourcc)
}

/// Whether the ACTIVE compositor may import `fourcc` at all — the single question
/// the draw path asks, so "not advertised" and "not importable" cannot drift.
pub fn may_import(r: &Registrar, fourcc: Fourcc) -> bool {
    catalog::expressible(fourcc, r.color_managed())
}
