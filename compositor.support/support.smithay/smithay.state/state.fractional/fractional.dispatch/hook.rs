use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use compositor_support_smithay_dispatch_state_base::fractional_base;
use compositor_support_smithay_dispatch_state_base::fractional_base::{Published, emit_to_surfaces};
use compositor_support_smithay_state_fractional_debounce::snap;

/// One pending entry's contribution to the batch fingerprint. Summed, not chained,
/// so the value is independent of `HashMap` iteration order.
fn stamp(id: &ObjectId, want: Published) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    want.scale.to_bits().hash(&mut hasher);
    want.idle.hash(&mut hasher);
    hasher.finish()
}

/// Publish each surface's fractional scale. `per_surface` is the already-aggregated
/// `(best_zoom, surface)` per surface across ALL outputs' viewports, so a window on
/// two monitors follows the sharper one; `idle` (disjoint from it) is mapped but
/// invisible everywhere and gets [`Published::IDLE`].
///
/// Everything differing from `sent` forms the PENDING BATCH, which is fingerprinted
/// and handed to `Fractional::tick`. Because the debounce observes the batch itself
/// rather than a proxy, one cycle covers every source of churn — zoom eases, drift
/// pans, pane-to-pane moves — and the batch is submitted whole once it settles.
/// A held surface keeps the scale the client ACTUALLY has, so a pan out-and-back
/// inside the quiet window resolves to no change and emits nothing at all.
///
/// Two RECORDED transitions bypass the gate: a surface with no [`Published`] record
/// (a new window) and one whose record is idle and which is visible again. A window
/// drifting into view must reach its real scale on the frame it appears. Returns the
/// surfaces a new VISIBLE scale reached — the windows about to re-lay themselves out,
/// which is what the caller re-states its decided size to.
pub fn emit_best_per_surface(
    fractional: &mut fractional_base::Fractional,
    sent: &mut HashMap<ObjectId, Published>,
    per_surface: &[(f64, WlSurface)],
    idle: &[WlSurface],
) -> Vec<WlSurface> {
    let cfg = fractional.cfg.clone();
    let mut emitted: Vec<WlSurface> = Vec::new();
    let desired = per_surface
        .iter()
        .map(|(zoom, surface)| (surface, Published::visible(snap(&cfg, zoom + cfg.auto_increment))))
        .chain(idle.iter().map(|surface| (surface, Published::IDLE)));

    let mut next: HashMap<ObjectId, Published> =
        HashMap::with_capacity(per_surface.len() + idle.len());
    let mut batch: Vec<(&WlSurface, Published)> = Vec::new();
    let mut fingerprint: u64 = 0;

    for (surface, want) in desired {
        let id = surface.id();
        let previous = sent.get(&id).copied();
        // Already holds this value; only the classification may differ, and
        // re-recording that costs no wire event.
        if previous.is_some_and(|p| p.scale == want.scale) {
            next.insert(id, want);
            continue;
        }
        // New window (no record) or un-idling (record says idle, visible now).
        if !want.idle && previous.map_or(true, |p| p.is_idle()) {
            emit_to_surfaces(want.scale, std::iter::once(surface));
            emitted.push(surface.clone());
            next.insert(id, want);
            continue;
        }
        // Held: keep the scale the client HAS, with the new classification. A
        // never-seen surface stays OUT of `sent` — recording a value it was never
        // told would suppress its emit forever.
        fingerprint = fingerprint.wrapping_add(stamp(&id, want));
        if let Some(previous) = previous {
            next.insert(id, Published { scale: previous.scale, idle: want.idle });
        }
        batch.push((surface, want));
    }

    if fractional.tick((!batch.is_empty()).then_some(fingerprint)) {
        for (surface, want) in batch {
            emit_to_surfaces(want.scale, std::iter::once(surface));
            if !want.idle { emitted.push(surface.clone()); }
            next.insert(surface.id(), want);
        }
    }
    // Seed for `new_surface` / `new_fractional_scale`. Still one global value: an
    // unmapped surface belongs to no pane, so there is no per-surface answer yet.
    if let Some(sharpest) = next.values().filter(|p| !p.idle).map(|p| p.scale).reduce(f64::max) {
        fractional.last_emitted_scale = Some(sharpest);
    }
    *sent = next;
    emitted
}
