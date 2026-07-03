//! Hit-testing — the `&Loop` rim entry points + the bbox (selection) API.
//!
//! The Loop-FREE core (`HitCx`, `surface_under_filtered_cx`, the per-drawable hit
//! helpers, `iced_camera_hcx`) lives in `compositor_y5_surface_interface_core` so
//! that Pass-1 input SYSTEMS can hit-test without the dependency cycle this crate
//! has via `orchestration_core` (whose focus accessors depend back on the system
//! crates). It is re-exported here so existing `surface_interface_base::hit::*`
//! callers are unchanged. interface.base keeps `orchestration_core` (for these
//! `&Loop` wrappers + the bbox path, which still reads `_loop.inner` directly).

use crate::position;
use smithay::desktop::{Window, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::Layer;
use compositor_y5_camera_transform_translate::transform::{Context as XformCtx, Transform as Xform};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_monitor_compositor_iced_base::{HandleId, IcedSpace, Transform as IcedTransform};

// The hit type, filter, context, and the `_cx` hit-test entry come from the
// Loop-free core; re-export so callers keep using `surface_interface_base::hit::*`.
pub use compositor_y5_surface_interface_core::hit::{
    HitCx, HitFilter, SurfaceHit, iced_camera_hcx, pass_all, surface_under_filtered_cx,
};

/// Rim `&Loop` wrapper: feeds the spatial world's storage to the Loop-free core.
pub fn iced_camera(_loop: &Loop) -> (IcedTransform, Size<f64, Physical>) {
    iced_camera_hcx(&HitCx::new(_loop.inner.spatial_storage()))
}

/// Rim `&Loop` wrapper over `surface_under_filtered_cx` (spatial world storage).
pub fn surface_under_filtered(
    _loop: &Loop,
    position_world: Point<f64, Logical>,
    filter: HitFilter,
) -> Option<SurfaceHit> {
    surface_under_filtered_cx(_loop.inner.spatial_storage(), position_world, filter)
}

// ─── bbox check / overlap ───────────────────────────────────────────

/// Type-coerce a Rectangle between smithay coordinate markers without
/// changing the underlying numbers. Used at the boundary of
/// `surfaces_check`'s callback, since the callback signature is fixed
/// to `Logical` but we feed it values in various coord systems
/// (always in matched pairs for a single call).
#[inline]
fn coerce_rect<K1, K2>(r: Rectangle<f64, K1>) -> Rectangle<f64, K2> {
    Rectangle::from_loc_and_size(
        Point::<f64, K2>::from((r.loc.x, r.loc.y)),
        Size::<f64, K2>::from((r.size.w, r.size.h)),
    )
}

fn surfaces_check_filtered<F>(
    _loop: &Loop,
    bbox_world: Rectangle<f64, Logical>,
    callback: F,
    filter: HitFilter,
) -> Vec<SurfaceHit>
where
    F: Fn(Rectangle<f64, Logical>, Rectangle<f64, Logical>) -> bool,
{
    let mut hits = Vec::new();
    let ctx: XformCtx = _loop.size_ctx_all();

    // Project the world bbox into the three coordinate systems we
    // compare against.
    let bbox_xform: Xform = (bbox_world, ctx).into();

    // Physical (real panel pixels) for iced screen items + iced world
    // items (iced's registry stores world × scale, physical-typed).
    let bbox_phys: Rectangle<f64, Physical> = bbox_xform.into();

    // Screen-logical (camera applied, top-left anchored) for layers
    // and for finding which output the bbox is on.
    let bbox_screen_logical: Rectangle<f64, Logical> = bbox_xform.into();

    let (iced_transform, iced_output_size) = iced_camera(_loop);

    // ── 1. Iced Screen-space ────────────────────────────────────────
    //
    // Items live in physical pixels. Compare bbox in physical.
    let active_output = _loop.inner.active_output_key();
    if let Some(registry) = &_loop.inner.surface().registry {
        for item in registry.iter().rev() {
            if item.space() != IcedSpace::Screen {
                continue;
            }
            // Output-bound surfaces (per-monitor capture overlays) are hit-tested
            // only on the monitor the cursor is on; the physical hit point is in
            // that output's local pixels, so an identically-anchored instance
            // bound to another output would otherwise match at the same coords.
            if item.output().is_some_and(|o| o != active_output.as_str()) {
                continue;
            }
            let item_rect_i = item.screen_rect(&iced_transform, iced_output_size);
            let item_rect = Rectangle::<f64, Physical>::from_loc_and_size(
                (item_rect_i.loc.x as f64, item_rect_i.loc.y as f64),
                (item_rect_i.size.w as f64, item_rect_i.size.h as f64),
            );

            if callback(coerce_rect(bbox_phys), coerce_rect(item_rect)) {
                let hit = SurfaceHit::Iced {
                    handle: item.handle_id(),
                    space: IcedSpace::Screen,
                    layer: item.layer,
                    screen_point: Point::from((
                        item_rect.loc.x + item_rect.size.w / 2.0,
                        item_rect.loc.y + item_rect.size.h / 2.0,
                    )),
                };
                if filter(&hit) {
                    hits.push(hit);
                }
            }
        }
    }

    // ── 2-7. Per-output layers + windows + iced world ───────────────

    let relevant_outputs: Vec<_> = _loop
        .inner.space_state()
        .state
        .outputs()
        .filter(|o| {
            _loop
                .inner.space_state()
                .state
                .output_geometry(o)
                .map(|g| g.to_f64().overlaps(bbox_screen_logical))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    for output in &relevant_outputs {
        let output_loc = match _loop.inner.space_state().state.output_geometry(output) {
            Some(g) => g.loc.to_f64(),
            None => continue,
        };

        // bbox relative to the output's top-left, in screen-logical.
        let output_relative_bbox = Rectangle::from_loc_and_size(
            bbox_screen_logical.loc - output_loc,
            bbox_screen_logical.size,
        );

        let compositor_output_size_logical = _loop
            .inner.space_state()
            .state
            .output_geometry(output)
            .unwrap()
            .size;

        let layer_map = layer_map_for_output(output);
        let check_layer_enabled = _loop.inner.select().Selection.len() > 0;

        let mut collect_layer = |layer_band: Layer, hits: &mut Vec<SurfaceHit>| {
            if !check_layer_enabled {
                return;
            }
            for layer_surface in layer_map.layers_on(layer_band).rev() {
                let location = position::layer_surface_position(
                    _loop,
                    layer_surface,
                    compositor_output_size_logical,
                );
                let surface_size = layer_surface.bbox().size;
                let geom = Rectangle::from_loc_and_size(location, surface_size).to_f64();

                if callback(output_relative_bbox, geom) {
                    let s = layer_surface.wl_surface().clone();
                    let hit = SurfaceHit::Layer {
                        Ice: Some(true),
                        layer: layer_band,
                        surface: s,
                        position_space: bbox_world.loc,
                    };
                    if filter(&hit) {
                        hits.push(hit);
                    }
                }
            }
        };

        collect_layer(Layer::Overlay, &mut hits);
        collect_layer(Layer::Top, &mut hits);

        // Windows — y5-world directly.
        for window in _loop.inner.space_state().state.elements() {
            let Some(window_geom) = _loop
                .inner.space_state()
                .state
                .element_geometry(window)
                .map(|g| g.to_f64())
            else {
                continue;
            };

            if !callback(bbox_world, window_geom) {
                continue;
            }
            let Some(toplevel) = window.toplevel() else {
                continue;
            };
            let s = toplevel.wl_surface().clone();
            let location = _loop
                .inner.space_state()
                .state
                .element_location(window)
                .unwrap_or_default();

            let hit = SurfaceHit::Window {
                window: window.clone(),
                surface: s,
                position: location.to_f64(),
            };
            if filter(&hit) {
                hits.push(hit);
            }
        }

        // Iced World items — stored as world × scale (physical-typed).
        // Compare in physical, where bbox_phys is also.
        if let Some(registry) = &_loop.inner.surface().registry {
            for item in registry.iter().rev() {
                if item.space() != IcedSpace::World {
                    continue;
                }
                let item_loc = item.location();
                let item_size = item.size();
                let target_phys = Rectangle::<f64, Physical>::from_loc_and_size(
                    (item_loc.x as f64, item_loc.y as f64),
                    (item_size.w as f64, item_size.h as f64),
                );

                if callback(coerce_rect(bbox_phys), coerce_rect(target_phys)) {
                    let screen_center = iced_transform.world_to_screen(
                        iced_output_size,
                        Point::from((
                            item_loc.x as f64 + item_size.w as f64 / 2.0,
                            item_loc.y as f64 + item_size.h as f64 / 2.0,
                        )),
                    );
                    let hit = SurfaceHit::Iced {
                        handle: item.handle_id(),
                        layer: item.layer,
                        space: IcedSpace::World,
                        screen_point: screen_center,
                    };
                    if filter(&hit) {
                        hits.push(hit);
                    }
                }
            }
        }

        collect_layer(Layer::Bottom, &mut hits);
        collect_layer(Layer::Background, &mut hits);
    }

    hits
}

// ─── Public bbox API ────────────────────────────────────────────────

// /// Surfaces whose geometry overlaps `bbox_world` (y5-world).
// pub fn surfaces_overlap(_loop: &mut Loop, bbox_world: Rectangle<f64, Logical>) -> Vec<SurfaceHit> {
//     surfaces_overlap_filtered(_loop, bbox_world, &pass_all)
// }

/// Like `surfaces_overlap` but only includes hits the filter accepts.
pub fn surfaces_overlap_filtered(
    _loop: &Loop,
    bbox_world: Rectangle<f64, Logical>,
    filter: HitFilter,
) -> Vec<SurfaceHit> {
    surfaces_check_filtered(
        _loop,
        bbox_world,
        |source, target| target.overlaps(source),
        filter,
    )
}

// /// Surfaces whose geometry is fully inside `bbox_world` (y5-world).
// pub fn surfaces_inside(_loop: &mut Loop, bbox_world: Rectangle<f64, Logical>) -> Vec<SurfaceHit> {
//     surfaces_inside_filtered(_loop, bbox_world, &pass_all)
// }

/// Like `surfaces_inside` but only includes hits the filter accepts.
pub fn surfaces_inside_filtered(
    _loop: &Loop,
    bbox_world: Rectangle<f64, Logical>,
    filter: HitFilter,
) -> Vec<SurfaceHit> {
    surfaces_check_filtered(
        _loop,
        bbox_world,
        |source, target| source.contains_rect(target),
        filter,
    )
}
