use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;
use std::collections::HashMap;
use compositor_support_smithay_dispatch_state_base::fractional_base;
use compositor_support_smithay_dispatch_state_base::fractional_base::emit_to_surfaces;
use compositor_support_smithay_state_fractional_debounce::snap;

pub fn hook(
    fractional: &mut fractional_base::Fractional,
    space: &Space<Window>,
    zoom: f64,
) -> Option<f64> {
    let Some(tick_updated_scale) = fractional.tick(zoom) else {
        return None;
    };

    let surfaces: Vec<_> = space.elements().filter_map(|w| w.wl_surface()).collect();

    let surfaces = surfaces.iter().map(|s| &**s);
    emit_to_surfaces(tick_updated_scale, surfaces);

    Some(tick_updated_scale)
}

/// Per-window fractional scale from a list of `(viewport_zoom, its_visible_windows)`.
/// A window may be visible in several viewports; the strategy is "highest
/// resolution wins" — each window's scale follows its HIGHEST-zoom viewport
/// (`snap(zoom + auto_increment)`). The debounce still gates re-emits off the
/// sharpest zoom in play, so we don't spam clients during a zoom animation.
pub fn hook_per_window(
    fractional: &mut fractional_base::Fractional,
    viewports: &[(f64, Vec<WlSurface>)],
) -> Option<f64> {
    let max_zoom = viewports.iter().map(|(z, _)| *z).fold(f64::NEG_INFINITY, f64::max);
    if !max_zoom.is_finite() {
        return None;
    }
    let fired = fractional.tick(max_zoom)?;

    // For each window, keep the highest zoom across the viewports it appears in.
    let mut per: HashMap<ObjectId, (f64, WlSurface)> = HashMap::new();
    for (zoom, surfaces) in viewports {
        for surface in surfaces {
            per.entry(surface.id())
                .and_modify(|e| {
                    if *zoom > e.0 {
                        *e = (*zoom, surface.clone());
                    }
                })
                .or_insert_with(|| (*zoom, surface.clone()));
        }
    }
    for (zoom, surface) in per.values() {
        let scale = snap(&fractional.cfg, zoom + fractional.cfg.auto_increment);
        emit_to_surfaces(scale, std::iter::once(surface));
    }
    Some(fired)
}

/// Emit each surface's best-resolution fractional scale, but ONLY when it changed
/// since the last emit (dedup via the caller-owned `sent` map). `per_surface` is the
/// already-aggregated `(best_zoom, surface)` per surface across ALL outputs' viewports
/// — the caller derives the cross-output max so a window on two monitors follows the
/// sharper one. Emit-on-change (not per frame) is what stops the per-output flip-flop
/// from re-sending `wp_fractional_scale` to clients every frame. `snap` quantises the
/// zoom to the scale lattice, so a smooth zoom only re-emits at lattice boundaries.
pub fn emit_best_per_surface(
    fractional: &fractional_base::Fractional,
    sent: &mut HashMap<ObjectId, f64>,
    per_surface: &[(f64, WlSurface)],
) {
    let cfg = &fractional.cfg;
    let mut next: HashMap<ObjectId, f64> = HashMap::with_capacity(per_surface.len());
    for (zoom, surface) in per_surface {
        let scale = snap(cfg, zoom + cfg.auto_increment);
        if sent.get(&surface.id()) != Some(&scale) {
            emit_to_surfaces(scale, std::iter::once(surface));
        }
        next.insert(surface.id(), scale);
    }
    *sent = next;
}
