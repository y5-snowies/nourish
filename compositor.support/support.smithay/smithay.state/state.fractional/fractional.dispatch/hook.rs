use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::collections::HashMap;
use compositor_support_smithay_dispatch_state_base::fractional_base;
use compositor_support_smithay_dispatch_state_base::fractional_base::emit_to_surfaces;
use compositor_support_smithay_state_fractional_debounce::snap;

/// Emit each surface's best-resolution fractional scale, but ONLY when it changed
/// since the last emit (dedup via the caller-owned `sent` map). `per_surface` is the
/// already-aggregated `(best_zoom, surface)` per surface across ALL outputs' viewports
/// — the caller derives the cross-output max so a window on two monitors follows the
/// sharper one. Emit-on-change (not per frame) is what stops the per-output flip-flop
/// from re-sending `wp_fractional_scale` to clients every frame. `snap` quantises the
/// zoom to the scale lattice, so a smooth zoom only re-emits at lattice boundaries.
///
/// Surfaces absent from BOTH lists (invisible windows without the opt-in) fall out
/// of `sent`, so no update reaches them until they reappear — at which point the
/// `None != scale` miss re-emits their real scale. `idle` (the opt-in: mapped but
/// invisible everywhere, disjoint from `per_surface`) instead gets scale 1 — below
/// the lattice floor, so it can never collide with a snapped zoom — letting the
/// client drop its hi-res buffers; staying in `sent` makes the 1 fire only once.
pub fn emit_best_per_surface(
    fractional: &fractional_base::Fractional,
    sent: &mut HashMap<ObjectId, f64>,
    per_surface: &[(f64, WlSurface)],
    idle: &[WlSurface],
) {
    let cfg = &fractional.cfg;
    let mut next: HashMap<ObjectId, f64> = HashMap::with_capacity(per_surface.len() + idle.len());
    for (zoom, surface) in per_surface {
        let scale = snap(cfg, zoom + cfg.auto_increment);
        if sent.get(&surface.id()) != Some(&scale) {
            emit_to_surfaces(scale, std::iter::once(surface));
        }
        next.insert(surface.id(), scale);
    }
    for surface in idle {
        if sent.get(&surface.id()) != Some(&1.0) {
            emit_to_surfaces(1.0, std::iter::once(surface));
        }
        next.insert(surface.id(), 1.0);
    }
    *sent = next;
}
