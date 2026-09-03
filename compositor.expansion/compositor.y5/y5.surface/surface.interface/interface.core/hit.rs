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

use smithay::desktop::{PopupManager, Window, WindowSurfaceType, layer_map_for_output};
use smithay::desktop::utils::under_from_surface_tree;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::Layer;
use smithay::backend::renderer::utils::{RendererSurfaceStateUserData, SurfaceView};
use smithay::wayland::compositor::{SurfaceData, with_states};
use smithay::wayland::seat::WaylandFocus;
use compositor_y5_camera_transform_translate::slot;
use compositor_y5_camera_transform_translate::fit::{self, WindowFit, window_fit};
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
use compositor_support_smithay_state_window_shell::shell;

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
    /// Compositor-drawn window chrome — the letterbox bars. Names the window but
    /// carries NO client surface, and the three consequences are all wanted:
    /// pointer motion finds no focus here so the client is sent a leave rather
    /// than an edge coordinate it never asked for; the press still resolves to
    /// the window, so `apply_focus` raises and focuses it and a Move/Scale grab
    /// anchors; and returning a hit at all stops the search, so a bar occludes
    /// whatever is beneath it instead of letting the click fall through.
    WindowChrome { window: Window },
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
            Self::Iced { .. } | Self::WindowChrome { .. } => None,
        }
    }

    pub fn position_motion(&self) -> Option<Point<f64, Logical>> {
        match self {
            Self::Window { position, .. } => Some(*position),
            Self::Layer { position_space, .. } => Some(*position_space),
            Self::Iced { .. } | Self::WindowChrome { .. } => None,
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
            Self::Window { window, .. } | Self::WindowChrome { window } => Some(window),
            _ => None,
        }
    }

    /// True when the hit is compositor chrome rather than client content, so
    /// nothing may be delivered to a client for it.
    pub fn is_chrome(&self) -> bool {
        matches!(self, Self::WindowChrome { .. })
    }

    /// True for a wlr layer-shell surface hit (used to gate layer input, e.g. make layers
    /// presentational while the overview overlay is open, like windows).
    pub fn is_layer(&self) -> bool {
        matches!(self, Self::Layer { .. })
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
    let cfg = compositor_model_environment_config_base::base::get();
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

    let slot_size = slot::expected_size(window);
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
            let fit = window_fit(
                elem_loc,
                geom,
                view_dst,
                slot_size,
                stretch,
            );
            let WindowFit { fit_sx, fit_sy, fit_surf, ref_size, cover: _ } = fit;
            let local = Point::<f64, Logical>::from((
                (position_world.x - fit_surf.0) / fit_sx,
                (position_world.y - fit_surf.1) / fit_sy,
            ));

            // Popups, in the SAME fit frame as the content, through the one mapping the
            // render path also uses (`fit::popup_offset`) — the two must agree or the
            // pointer lands somewhere the menu is not drawn.
            //
            // Walked here rather than through `Window::surface_under(.., POPUP)`, which
            // cannot see an X11 popup: smithay's X11 arm answers only when the flags
            // contain `TOPLEVEL`, so a `POPUP`-only ask returns `None` for every X11
            // window. The hit then fell through to the parent toplevel and delivered
            // PARENT-local coordinates. The menu still received events, because the X
            // server routes them into it by root coordinate, but measured through the
            // wrong mapping: the pointer did not line up with what the menu drew.
            //
            // That only ever affected the `cover` regime, which is a per-frame verdict
            // and not a property of an app (`window_fit` recomputes it from geometry
            // against slot every frame) — so it was every X11 window whenever its surface
            // fitted its slot, the ordinary steady state, while the same window
            // hit-tested its menu correctly the rest of the time. Hence "intermittent".
            //
            // `POPUP | SUBSURFACE` so a popup's own subsurfaces are reachable; the
            // oversized branch this replaces already passed both.
            if let Some(root) = &root_surface {
                for (popup, pop_loc) in PopupManager::popups_for_surface(root) {
                    let off = fit::popup_offset(&fit, geom, pop_loc, popup.geometry().loc);
                    let off = Point::<i32, Logical>::from((
                        off.x.round() as i32,
                        off.y.round() as i32,
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

            // Letterbox bars: inside the SLOT but outside the fitted content.
            //
            // The slot is the window's authoritative extent — the decoration
            // frames it, the renderer crops to it, and the black fill paints it
            // (`window.draw.frame::scene`). No surface reaches into the bars, so
            // `surface_under` above misses and the press used to fall through to
            // the canvas: clicking a bar hit nothing, and a move/resize grab
            // never anchored. The bars are opaque compositor pixels, so they hit
            // the window — as `WindowChrome`, which deliberately carries no
            // surface, so the client is told nothing about a pointer that is not
            // over it. See the variant for what that buys.
            //
            // Scoped to the bars, not the whole slot, on purpose: a miss INSIDE
            // the content is the client's own input region talking, and that
            // still falls through exactly as before. The content rect is the
            // one `window_fit` centres in the slot, so in the cover and stretch
            // regimes it covers the slot and this can never fire — only the
            // contain (genuine letterbox) regime reaches here.
            let content = Rectangle::<f64, Logical>::new(
                Point::from((
                    elem_loc.x as f64 + (slot_size.w as f64 - ref_size.w as f64 * fit_sx) / 2.0,
                    elem_loc.y as f64 + (slot_size.h as f64 - ref_size.h as f64 * fit_sy) / 2.0,
                )),
                Size::from((ref_size.w as f64 * fit_sx, ref_size.h as f64 * fit_sy)),
            );
            let slot_rect = Rectangle::<f64, Logical>::new(
                Point::from((elem_loc.x as f64, elem_loc.y as f64)),
                Size::from((slot_size.w as f64, slot_size.h as f64)),
            );
            // `root_surface` is required: with no root the render path draws only
            // the decoration and never paints the fill, so there are no bars.
            if root_surface.is_some()
                && slot_rect.contains(position_world)
                && !content.contains(position_world)
            {
                let hit = SurfaceHit::WindowChrome { window: window.clone() };
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

    // For layer-shell: screen-logical (camera applied, top-left anchored per output).
    // `cursor_xform` was projected through `pane_context()` — the CURSOR's output — so
    // this point is in THAT output's LOCAL, 0-based logical pixels, not global multi-
    // output space. Hit-test the SAME output's layer map with the local point (mirrors
    // the iced-screen path, whose `screen_point` is likewise output-local).
    //
    // The old code re-derived the output by testing this LOCAL point against GLOBAL
    // `output_geometry`; the two coincide only on the output at the global origin, so a
    // layer surface on any other monitor got the wrong output's layer map and silently
    // received no pointer input (the multi-monitor layer-shell input bug).
    let cursor_logical: Point<f64, Logical> = cursor_xform.into();
    let position_screen = cursor_logical;
    let output = hcx.current_output();
    let output_pos = position_screen;
    let layer_map = layer_map_for_output(output);

    // Hit-test each layer at its natural z-band. Layer ordering (Overlay/Top
    // above windows, Bottom/Background below — see the caller order) is what
    // keeps a background/bottom surface from stealing clicks from windows, so no
    // extra gating is needed: a Top/Overlay dock is interactive, a Background
    // wallpaper is not reachable while a window covers it. Pointer reachability
    // is further narrowed by the surface's own input region inside
    // `surface_under`.
    let check_layer = |layer_band: Layer| -> Option<SurfaceHit> {
        for layer_surface in layer_map.layers_on(layer_band).rev() {
            // smithay's arranged geometry (honors anchor/margin/exclusive/size); the
            // interactive rect includes popups so a menu off the bar is clickable.
            let Some(geo) = layer_map.layer_geometry(layer_surface) else {
                continue;
            };
            // Containment (broad phase) includes popups so a click on a menu off the bar
            // counts — `bbox_with_popups.loc` is negative when a popup extends up/left.
            let popups = layer_surface.bbox_with_popups();
            let hit_rect = Rectangle::from_loc_and_size(geo.loc + popups.loc, popups.size);

            if !hit_rect.to_f64().contains(output_pos) {
                continue;
            }

            let surface_local = output_pos - geo.loc.to_f64();
            // `sub_pos` is the HIT surface's top-left relative to the layer surface origin:
            // (0,0) for the bar itself, non-zero for a popup (a menu/submenu offset from the
            // bar). The pointer-focus origin below MUST be this surface's origin, not the
            // layer's — otherwise a popup receives motion/clicks measured from the bar and,
            // once offset far enough, the local coords land outside it and the client drops
            // the events (the "some popups get no pointer input" bug).
            let Some((s, sub_pos)) =
                layer_surface.surface_under(surface_local, WindowSurfaceType::ALL)
            else {
                continue;
            };

            let surface_origin_space = geo.loc.to_f64() + sub_pos.to_f64();
            let unscaled = position_screen - surface_origin_space;

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
    // `is_drawn` is a PRECONDITION here, not one of the caller's filters. A Space element
    // is not automatically something on screen — an X11 unmap is a hide (the `Window`
    // stays, its `wl_surface` cleared) and an xdg toplevel that commits a null buffer
    // keeps its element with an empty bbox — while its SLOT survives either way
    // (`expected_size` is `Decided` from map and nothing clears it). So the letterbox
    // branch below would find the whole slot outside a zero-sized content rect and answer
    // `WindowChrome`, swallowing every click over a window that is not there.
    //
    // Applied to the map rather than to each caller's closure because every input device
    // reaches this driver — pointer, touch, tablet, the lock seat and the canvas systems
    // all call `surface_under_filtered` — and their closures express POLICY (overview
    // open, the carried window), which this is not. It removes the window's popups with
    // it, correctly: the render path builds those inside the per-window render, so a
    // window outside `vis.drawn` has undrawn popups, and hit and render stay in step.
    let by_uuid: std::collections::HashMap<uuid::Uuid, Window> = hcx.space_state().state
        .elements()
        .filter_map(|w| {
            if !compositor_support_smithay_state_window_ident::ident::is_drawn(w) {
                return None;
            }
            compositor_y5_window_interface_record::window::LoopWindow::uuid(w).map(|u| (u, w.clone()))
        })
        .collect();
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

