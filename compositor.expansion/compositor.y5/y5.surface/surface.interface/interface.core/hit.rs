//! Hit-testing for surfaces under the cursor (or under a bbox).
//!
//! Priority order, topmost first:
//! 1. Iced (Screen space)         — overlays, HUD, system menus
//! 2. Layer Overlay
//! 3. Layer Top
//! 4. Windows
//! 5. Iced (World space)          — placeholders, scene-anchored UIs
//! 6. Layer Bottom
//! 7. Layer Background
//!
//! ## Coordinate model
//!
//! Each surface category has its native coordinate space for hit-testing.
//! We project the input bbox into that space before comparing:
//!
//! - **Iced Screen** items live in **physical pixels** (real panel pixels,
//!   not following the camera). We project the input bbox via Transform
//!   and compare in physical.
//! - **Iced World** items live in **world × scale** (physical-typed,
//!   camera applied by iced internally). Compare bbox in physical.
//! - **Layer-shell** items are anchored to the output's top-left in
//!   **screen-logical** coords (camera applied, divided by scale).
//! - **Windows** live in **y5-world** (smithay's Space stores world by
//!   convention). Direct world comparison.
//!
//! The geometry-overlap callback takes `Rectangle<f64, Logical>` for both
//! sides but the unit is "whatever both sides are typed as" — we coerce
//! at the boundary. Both sides of any single callback invocation are in
//! the same space.

use crate::position;
use smithay::desktop::{PopupManager, Window, WindowSurfaceType, layer_map_for_output};
use smithay::desktop::utils::under_from_surface_tree;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::Layer;
use smithay::backend::renderer::utils::{RendererSurfaceStateUserData, SurfaceView};
use smithay::wayland::compositor::{SurfaceData, with_states};
use smithay::wayland::seat::WaylandFocus;
use compositor_y5_camera_transform_translate::slot;
use compositor_y5_camera_transform_translate::fit::{WindowFit, window_fit};
use compositor_y5_camera_transform_translate::transform::{
    Context as XformCtx, Transform as Xform,
};

/// Read a surface's [`SurfaceView`] logical destination size (reflects viewport / buffer-scale).
fn root_dst(surface: &WlSurface) -> Option<smithay::utils::Size<i32, Logical>> {
    with_states(surface, |states: &SurfaceData| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .and_then(|m| m.lock().ok().and_then(|g| g.view()))
            .map(|v: SurfaceView| v.dst)
    })
}
use compositor_monitor_compositor_iced_base::{HandleId, IcedSpace, Transform as IcedTransform};
use compositor_support_system_storage_slot_base::base::Storage;

// ─── Hit context ────────────────────────────────────────────────────
//
// Read-only data source for the `surface_under_filtered` path: ONE world's
// `Storage`. The rim builds it from the spatial/spawn-target world
// (`spatial_storage()`); a Pass-1 input system builds it from `cx.storage` (its
// active world). The hit-test reads only through these accessors — and only
// reads — so the same logic serves both callers identically without the mutable
// `Space` hatch. Mirrors the `Orchestrator` focus accessors (state.rs), which
// read the same tokens from the same world storage.

pub struct HitCx<'a> {
    storage: &'a Storage,
}

impl<'a> HitCx<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    fn space_state(&self) -> &compositor_support_smithay_state_space_base::state::SpaceState {
        &self.storage.get(&compositor_support_world_host_space_base::base::SPACE).inner
    }

    /// The output the CURSOR is on — the monitor whose mode/scale the reverse
    /// projections (screen-space hit-testing) must use, so a physical cursor on a
    /// secondary monitor is mapped against THAT monitor, not the primary. Resolved
    /// from `OUTPUT_VIEWS.current` (the key the pointer path keeps in sync with the
    /// cursor's output) by matching the same "make model serial" `output_key` the
    /// render/input paths use; falls back to the first output pre-identity.
    fn current_output(&self) -> &smithay::output::Output {
        let key = &self.storage.get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS).current;
        self.space_state()
            .state
            .outputs()
            .find(|o| {
                let p = o.physical_properties();
                format!("{} {} {}", p.make, p.model, p.serial_number) == *key
            })
            .or_else(|| self.space_state().state.outputs().next())
            .unwrap_or_else(|| abort!("no output for hit-test"))
    }

    /// The `output_key` of the monitor the cursor is on — used to hit-test
    /// output-bound screen surfaces (per-monitor capture overlays) only on the
    /// monitor whose local pixels the screen point was projected into.
    fn current_output_key(&self) -> String {
        let p = self.current_output().physical_properties();
        format!("{} {} {}", p.make, p.model, p.serial_number)
    }

    fn camera(&self) -> &compositor_y5_camera_state_base::state::Camera {
        self.storage.get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS).current_views().focus_camera()
    }

    fn surface(&self) -> &compositor_y5_surface_state_base::state::SurfaceState {
        self.storage.get(&compositor_y5_surface_system_base::base::SURFACE)
    }

    fn select(&self) -> &compositor_y5_select_state_base::select::CanvasSelect {
        self.storage.get(&compositor_y5_select_state_base::select::SELECT)
    }

    fn drawable_order(&self) -> Vec<uuid::Uuid> {
        self.storage
            .get(&compositor_support_world_order_track_base::base::DRAW_ORDER)
            .ordered()
            .iter()
            .rev()
            .map(|(id, _)| id.0)
            .collect()
    }

    fn size_ctx_all(&self) -> XformCtx {
        let output = self.current_output();
        let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
        let scale = output.current_scale().fractional_scale();
        let camera = &self.camera().transform;
        XformCtx::new(
            (camera.position.x, camera.position.y),
            camera.zoom,
            (mode.size.w as f64, mode.size.h as f64),
            scale,
        )
    }

    /// Region context for the pane under the cursor (the `pointer` slot). Projects
    /// the pane-mapped world cursor back to the TRUE physical/logical position —
    /// the analog of the renderer's pane context — so screen-space hit-testing
    /// (iced screen, layer-shell) lands where the cursor actually is when split.
    fn pane_context(&self) -> XformCtx {
        let output = self.current_output();
        let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
        let scale = output.current_scale().fractional_scale();
        let viewports = self.storage.get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS).current_views();
        let bounds = smithay::utils::Rectangle::new(smithay::utils::Point::from((0, 0)), mode.size);
        let computed = compositor_y5_viewport_layout_base::layout::compute(viewports, bounds);
        let rect = computed.regions.iter().find(|r| r.slot == viewports.pointer).map(|r| r.rect).unwrap_or(bounds);
        let camera = &self.camera().transform;
        XformCtx::new_region(
            (camera.position.x, camera.position.y),
            camera.zoom,
            (rect.loc.x as f64 / scale, rect.loc.y as f64 / scale),
            (rect.size.w as f64, rect.size.h as f64),
            scale,
        )
    }
}

// ─── Hit type ───────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SurfaceHit {
    Window {
        window: Window,
        surface: WlSurface,
        position: Point<f64, Logical>,
    },
    Layer {
        Ice: Option<bool>,
        layer: Layer,
        surface: WlSurface,
        position_space: Point<f64, Logical>,
    },
    Iced {
        handle: HandleId,
        space: IcedSpace,
        layer: u64,
        screen_point: Point<f64, Physical>,
    },
}

impl SurfaceHit {
    pub fn surface(&self) -> Option<&WlSurface> {
        match self {
            Self::Window { surface, .. } | Self::Layer { surface, .. } => Some(surface),
            Self::Iced { .. } => None,
        }
    }

    pub fn position_motion(&self) -> Option<Point<f64, Logical>> {
        match self {
            Self::Window { position, .. } => Some(*position),
            Self::Layer { position_space, .. } => Some(*position_space),
            Self::Iced { .. } => None,
        }
    }

    pub fn screen_point(&self) -> Option<Point<f64, Physical>> {
        match self {
            Self::Iced { screen_point, .. } => Some(*screen_point),
            _ => None,
        }
    }

    pub fn window(&self) -> Option<&Window> {
        match self {
            Self::Window { window, .. } => Some(window),
            _ => None,
        }
    }

    pub fn ice(&self) -> Option<bool> {
        match self {
            Self::Layer { Ice, .. } => Ice.clone(),
            _ => None,
        }
    }

    pub fn iced_handle(&self) -> Option<HandleId> {
        match self {
            Self::Iced { handle, .. } => Some(*handle),
            _ => None,
        }
    }

    pub fn iced_layer(&self) -> Option<u64> {
        match self {
            Self::Iced { layer, .. } => Some(*layer),
            _ => None,
        }
    }

    pub fn iced_space(&self) -> Option<IcedSpace> {
        match self {
            Self::Iced { space, .. } => Some(*space),
            _ => None,
        }
    }

    pub fn is_window(&self) -> bool {
        matches!(self, Self::Window { .. })
    }

    pub fn is_iced(&self) -> bool {
        matches!(self, Self::Iced { .. })
    }
}

// ─── Filter type ────────────────────────────────────────────────────
//
// A filter is a predicate that decides whether a candidate SurfaceHit
// should be included in the result. It runs as the search visits each
// candidate, so for `surface_under_*` the filter lets the search
// continue past a rejected hit instead of returning it.
//
// For `surfaces_*` (bbox variants), the filter just decides whether
// a discovered hit goes into the result vec.
//
// `pass_all` is the default; existing API delegates to filtered API
// with `pass_all`.

pub type HitFilter<'a> = &'a dyn Fn(&SurfaceHit) -> bool;

#[inline]
pub fn pass_all(_: &SurfaceHit) -> bool {
    true
}

// ─── Iced camera helper ─────────────────────────────────────────────

pub fn iced_camera_hcx(hcx: &HitCx) -> (IcedTransform, Size<f64, Physical>) {
    let output = hcx.current_output();
    let mode = output.current_mode().unwrap_or_else(|| abort!("output has mode"));
    let scale = output.current_scale().fractional_scale();

    let cam = &hcx.camera().transform;
    let transform = IcedTransform {
        position: Point::from((cam.position.x * scale, cam.position.y * scale)),
        zoom: *cam.zoom(),
    };
    let output_size = Size::<f64, Physical>::from((mode.size.w as f64, mode.size.h as f64));
    (transform, output_size)
}

fn hit_iced_in_space(
    hcx: &HitCx,
    screen_point: Point<f64, Physical>,
    transform: &IcedTransform,
    output_size: Size<f64, Physical>,
    space: IcedSpace,
    filter: HitFilter,
) -> Option<SurfaceHit> {
    let reg = hcx.surface().registry.as_ref()?;
    let active_output = hcx.current_output_key();
    for item in reg.iter().rev() {
        if item.space() != space {
            continue;
        }
        // Output-bound surfaces (per-monitor capture overlays) hit-test only on
        // the cursor's monitor: `screen_point` is in that output's local pixels,
        // so an identically-anchored instance bound to another output would match
        // at the same coordinates.
        if item.output().is_some_and(|o| o != active_output.as_str()) {
            continue;
        }
        // Capture border/dim are hit-test-transparent: skip them entirely so the
        // pointer (motion + press + focus) reaches the window beneath — true
        // passthrough while recording, not just a non-swallowed press.
        if (item.layer & compositor_orchestration_draw_layer_base::base::Layer::CAPTURE_PASSTHROUGH.bits()) != 0 {
            continue;
        }
        if !item.contains_screen_point(screen_point, transform, output_size) {
            continue;
        }
        let hit = SurfaceHit::Iced {
            layer: item.layer,
            handle: item.handle_id(),
            space,
            screen_point,
        };
        if filter(&hit) {
            return Some(hit);
        }
    }
    None
}

/// Hit-test ONE world iced surface by id (the per-id form used in the unified,
/// DrawOrder-ordered content hit-test — parity with the unified draw).
fn hit_iced_one(
    hcx: &HitCx,
    id: HandleId,
    screen_point: Point<f64, Physical>,
    transform: &IcedTransform,
    output_size: Size<f64, Physical>,
    filter: HitFilter,
) -> Option<SurfaceHit> {
    let reg = hcx.surface().registry.as_ref()?;
    let item = reg.get(id)?;
    if item.space() != IcedSpace::World {
        return None;
    }
    if (item.layer & compositor_orchestration_draw_layer_base::base::Layer::CAPTURE_PASSTHROUGH.bits()) != 0 {
        return None;
    }
    if !item.contains_screen_point(screen_point, transform, output_size) {
        return None;
    }
    let hit = SurfaceHit::Iced { layer: item.layer, handle: item.handle_id(), space: IcedSpace::World, screen_point };
    filter(&hit).then_some(hit)
}

/// Hit-test ONE window at `position_world` (the window drawable's hit). Inverts
/// the exact fit the renderer applies in `window.draw.frame::scene` so input
/// stays locked to what is drawn; popups first (on top).
fn hit_window(
    hcx: &HitCx,
    window: &Window,
    position_world: Point<f64, Logical>,
    filter: HitFilter,
) -> Option<SurfaceHit> {
    let cfg = compositor_developer_environment_config_base::base::get();
    let elem_loc = hcx.space_state().state.element_location(window).unwrap_or_default();
    let geom = window.geometry();
    let gloc = geom.loc;

    // Craft `position` so smithay (`event.location − position`) delivers exactly
    // the surface-local coordinate the client expects.
    let deliver = |surface: WlSurface, sub_pos: Point<i32, Logical>, local: Point<f64, Logical>| {
        let surface_local =
            Point::<f64, Logical>::from((local.x - sub_pos.x as f64, local.y - sub_pos.y as f64));
        let position = Point::<f64, Logical>::from((
            position_world.x - surface_local.x,
            position_world.y - surface_local.y,
        ));
        SurfaceHit::Window { window: window.clone(), surface, position }
    };

    let slot_size = if cfg.window_client_size_fallback {
        window
            .toplevel()
            .and_then(|t| t.with_pending_state(|s| s.size))
            .filter(|s| s.w > 0 && s.h > 0)
            .or_else(|| Some(geom.size))
    } else {
        slot::expected_size(window)
    };
    let root_surface = window.wl_surface().map(|c| c.into_owned());

    match slot_size.filter(|s| s.w > 0 && s.h > 0) {
        None => {
            let local = Point::<f64, Logical>::from((
                position_world.x - (elem_loc.x - gloc.x) as f64,
                position_world.y - (elem_loc.y - gloc.y) as f64,
            ));
            if let Some((surface, sub_pos)) = window.surface_under(local, WindowSurfaceType::ALL) {
                let hit = deliver(surface, sub_pos, local);
                if filter(&hit) {
                    return Some(hit);
                }
            }
        }
        Some(slot_size) => {
            let view_dst = root_surface.as_ref().and_then(root_dst).unwrap_or(geom.size);
            let stretch = slot::resize_stretching(window, geom.size);
            let WindowFit { fit_sx, fit_sy, fit_surf, ref_size, cover } = window_fit(
                elem_loc,
                geom,
                view_dst,
                window.bbox(),
                slot_size,
                cfg.window_subsurface_shrinks,
                stretch,
            );
            let local = Point::<f64, Logical>::from((
                (position_world.x - fit_surf.0) / fit_sx,
                (position_world.y - fit_surf.1) / fit_sy,
            ));

            if cover {
                if let Some((surface, sub_pos)) =
                    window.surface_under(local, WindowSurfaceType::POPUP)
                {
                    let hit = deliver(surface, sub_pos, local);
                    if filter(&hit) {
                        return Some(hit);
                    }
                }
            } else if let Some(root) = &root_surface {
                let psx = if geom.size.w > 0 { ref_size.w as f64 / geom.size.w as f64 } else { 1.0 };
                let psy = if geom.size.h > 0 { ref_size.h as f64 / geom.size.h as f64 } else { 1.0 };
                for (popup, pop_loc) in PopupManager::popups_for_surface(root) {
                    let pg = popup.geometry().loc;
                    let off = Point::<i32, Logical>::from((
                        ((pop_loc.x - pg.x) as f64 * psx).round() as i32,
                        ((pop_loc.y - pg.y) as f64 * psy).round() as i32,
                    ));
                    if let Some((surface, sub_pos)) = under_from_surface_tree(
                        popup.wl_surface(),
                        local,
                        off,
                        WindowSurfaceType::POPUP | WindowSurfaceType::SUBSURFACE,
                    ) {
                        let hit = deliver(surface, sub_pos, local);
                        if filter(&hit) {
                            return Some(hit);
                        }
                    }
                }
            }

            if let Some((surface, sub_pos)) =
                window.surface_under(local, WindowSurfaceType::TOPLEVEL | WindowSurfaceType::SUBSURFACE)
            {
                let hit = deliver(surface, sub_pos, local);
                if filter(&hit) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

/// A first-class drawable in the content band: it OWNS its hit-testing (and, as
/// the model grows, its draw). The driver walks the drawable order (raise +
/// layer) and dispatches here — no window-vs-iced branching in the driver. New
/// kinds (group, bevy) add a variant; a Group could decide its hit from its own
/// + system state rather than pure geometry.
enum Drawable {
    Window(Window),
    IcedWorld(HandleId),
}

impl Drawable {
    fn hit(
        &self,
        hcx: &HitCx,
        position_world: Point<f64, Logical>,
        cursor_phys: Point<f64, Physical>,
        iced_transform: &IcedTransform,
        iced_output_size: Size<f64, Physical>,
        filter: HitFilter,
    ) -> Option<SurfaceHit> {
        match self {
            Drawable::Window(w) => hit_window(hcx, w, position_world, filter),
            Drawable::IcedWorld(h) => {
                hit_iced_one(hcx, *h, cursor_phys, iced_transform, iced_output_size, filter)
            }
        }
    }
}

// ─── surface_under ─────────────────────────────────────────────────

/// Hit-test a world point. Returns the topmost surface at that point.
///
/// `position_world` is in y5-world: same units as `space.element_location`,
/// camera-independent.
// pub fn surface_under(_loop: &Loop, position_world: Point<f64, Logical>) -> Option<SurfaceHit> {
//     surface_under_filtered(_loop, position_world, &pass_all)
// }

/// Hit-test a world point with a filter predicate, over a world's raw `Storage`.
/// The filter is applied as each candidate hit is discovered, so the search keeps
/// going past rejected candidates rather than returning early. A Pass-1 input
/// system passes `cx.storage`; the rim's `&Loop` wrapper (interface.base) feeds the
/// spatial world's storage.
pub fn surface_under_filtered_cx(
    storage: &Storage,
    position_world: Point<f64, Logical>,
    filter: HitFilter,
) -> Option<SurfaceHit> {
    let hcx = HitCx::new(storage);

    // World-iced items render through the full-output camera, so hit them with the
    // full-output projection (keeps render and hit consistent for those).
    let cursor_phys_world: Point<f64, Physical> = {
        let x: Xform = (position_world, hcx.size_ctx_all()).into();
        x.into()
    };

    // Screen-space items (iced screen, layer-shell) are full-screen; project the
    // pane-mapped world cursor back to its TRUE physical/logical via the pane
    // context so the hit lands where the cursor actually is when split.
    let cursor_xform: Xform = (position_world, hcx.pane_context()).into();
    let cursor_phys: Point<f64, Physical> = cursor_xform.into();

    let (iced_transform, iced_output_size) = iced_camera_hcx(&hcx);

    // 1. Iced Screen-space items — topmost.
    if let Some(hit) = hit_iced_in_space(
        &hcx,
        cursor_phys,
        &iced_transform,
        iced_output_size,
        IcedSpace::Screen,
        filter,
    ) {
        return Some(hit);
    }

    // For layer-shell: screen-logical (camera applied, top-left anchored
    // per output).
    let cursor_logical: Point<f64, Logical> = cursor_xform.into();
    let position_screen = cursor_logical;

    // Find which output the cursor is on. output_geometry is logical.
    let output = hcx.space_state().state.outputs().find(|o| {
        hcx.space_state()
            .state
            .output_geometry(o)
            .map(|g| g.to_f64().contains(position_screen))
            .unwrap_or(false)
    })?;

    let output_loc = hcx.space_state()
        .state
        .output_geometry(output)
        .map(|g| g.loc)?;

    let output_pos = position_screen - output_loc.to_f64();

    let layer_map = layer_map_for_output(output);
    let check_layer_enabled = hcx.select().Selection.len() > 0;

    // Size of the output the cursor is actually on (found just above), not the
    // primary — layer-shell positioning on a secondary monitor must use its size.
    let compositor_output_size_logical = hcx.space_state()
        .state
        .output_geometry(output)
        .unwrap()
        .size;

    let check_layer = |layer_band: Layer| -> Option<SurfaceHit> {
        if !check_layer_enabled {
            return None;
        }
        for layer_surface in layer_map.layers_on(layer_band).rev() {
            let location = position::layer_surface_position_core(
                position_world,
                hcx.size_ctx_all(),
                layer_surface,
                compositor_output_size_logical,
            );
            let surface_size = layer_surface.bbox().size;
            let geom = Rectangle::from_loc_and_size(location, surface_size);

            if !geom.to_f64().contains(output_pos) {
                continue;
            }

            let surface_local = output_pos - geom.loc.to_f64();
            let Some((s, _sub_pos)) =
                layer_surface.surface_under(surface_local, WindowSurfaceType::ALL)
            else {
                continue;
            };

            let layer_origin_space = geom.loc.to_f64() + output_loc.to_f64();
            let unscaled = position_screen - layer_origin_space;

            let hit = SurfaceHit::Layer {
                Ice: Some(true),
                layer: layer_band,
                surface: s,
                position_space: position_world - unscaled,
            };
            if filter(&hit) {
                return Some(hit);
            }
        }
        None
    };

    // 2-3. Overlay → Top
    if let Some(hit) = check_layer(Layer::Overlay) {
        return Some(hit);
    }
    if let Some(hit) = check_layer(Layer::Top) {
        return Some(hit);
    }

    // 4. Windows — invert the exact fit the renderer applies in `window.draw.frame::scene`, so
    //    the cursor stays locked to what is drawn. The toplevel content is fitted
    //    (`view.dst → slot`) by `(cursor − fit_surf)/fit_s`; popups share that fit frame but
    //    additionally have their geometry-relative offset scaled PROPORTIONALLY by
    //    `ref_size/geom` (so a popup pins to the visible content, not the smaller declared
    //    geometry — mirrors the renderer). Popups are hit-tested first (on top). Topmost-first.
    // Content band: each drawable OWNS its hit; the driver walks the drawable
    // order (raise + layer) and dispatches via `Drawable` — windows and world
    // iced interleave here by raise, no kind-branching in the driver. Any window
    // not in the order (defensive) is hit-tested at the bottom.
    let order = hcx.drawable_order();
    let by_uuid: std::collections::HashMap<uuid::Uuid, Window> = hcx.space_state().state
        .elements().filter_map(|w| compositor_y5_window_interface_record::window::LoopWindow::uuid(w).map(|u| (u, w.clone()))).collect();
    let in_order: std::collections::HashSet<uuid::Uuid> = order.iter().copied().collect();
    for id in &order {
        let drawable = match by_uuid.get(id) {
            Some(w) => Drawable::Window(w.clone()),
            None => Drawable::IcedWorld(HandleId(id.as_u128() as u64)),
        };
        if let Some(hit) = drawable.hit(&hcx, position_world, cursor_phys_world, &iced_transform, iced_output_size, filter) {
            return Some(hit);
        }
    }
    for (u, w) in &by_uuid {
        if !in_order.contains(u) {
            if let Some(hit) = Drawable::Window(w.clone()).hit(&hcx, position_world, cursor_phys_world, &iced_transform, iced_output_size, filter) {
                return Some(hit);
            }
        }
    }

    // 6-7. Bottom → Background
    if let Some(hit) = check_layer(Layer::Bottom) {
        return Some(hit);
    }
    check_layer(Layer::Background)
}

