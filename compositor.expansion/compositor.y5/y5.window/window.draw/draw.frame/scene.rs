//! Per-window scene assembly.
//!
//! We do **not** use smithay's `Window::render_elements()` for the toplevel. We render the
//! toplevel surface tree natively (`render_elements_from_surface_tree`) and then place it into
//! the window's compositor-decided **slot** using smithay's own element utils
//! (`RescaleRenderElement` + `RelocateRenderElement` + `CropRenderElement`):
//! - the window's **geometry** is aspect-fit into the slot (never stretched), centered, and
//!   **cropped** to the slot so nothing spills out; a black letterbox fills the rest;
//! - **subsurfaces** scale/position with the toplevel (default) — they don't shrink it;
//! - **popups** get the *same* fit transform (so they sit at proprietary places within the
//!   surface, not the raw slot) but are cropped to the **output**, not the slot, so they may
//!   extend past the window. Popups never change the window's size.
//!
//! The camera (pan/zoom/scale) is applied by composing it into the rescale factor and the
//! relocate point (see `TRANSFORM.md`): a world point projects to physical via `Transform`,
//! and `physical = world*scale*zoom + (center - cam*zoom*scale)`. Input (`hit.rs`) inverts the
//! same fit. See `slot`/`Fit` and the authoritative-sizing plan.

use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::utils::{
    CropRenderElement, Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::{CommitCounter, RendererSurfaceStateUserData, SurfaceView};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::{PopupKind, PopupManager, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size};
use smithay::wayland::compositor::{SurfaceData, with_states};
use smithay::wayland::seat::WaylandFocus;
use compositor_y5_camera_transform_translate::slot;
use compositor_y5_camera_transform_translate::fit::{WindowFit, window_fit};
use compositor_y5_camera_transform_translate::transform::{Context as XformCtx, Transform};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_window_draw_element::element::{ClampOpaque, Element, ElementWindowSurface};
use compositor_y5_window_interface_draw::visible::DrawWindow;
use compositor_y5_window_interface_record::window::LoopWindow;

/// Read a surface's [`SurfaceView`] (src crop / dst size / subsurface offset), if mapped.
fn view_of(states: &SurfaceData) -> Option<SurfaceView> {
    states
        .data_map
        .get::<RendererSurfaceStateUserData>()
        .and_then(|m| m.lock().ok().and_then(|g| g.view()))
}

/// Logical `dst` size of a (popup) surface, used to keep IME popups fully on-screen.
fn popup_dst(surface: &WlSurface) -> Option<Size<i32, Logical>> {
    with_states(surface, |s| view_of(s)).map(|v| v.dst)
}

fn project_point(ctx: XformCtx, x: f64, y: f64) -> Point<i32, Physical> {
    let t: Transform = ((x, y), ctx).into();
    t.into()
}
/// Project a world rect to physical by its **corners** — each corner rounded once — so an edge at
/// a fixed world coordinate lands at a fixed screen coordinate. Projecting `loc` then adding a
/// separately-rounded `size*zoom` makes `round(left) + round(width)` wobble ±1px even when the
/// world right edge is constant (the resize-from-left jitter). The decoration projects the slot
/// the same way (`bound::calculate`), so content crop and border track each other exactly.
fn project_rect(ctx: XformCtx, x: f64, y: f64, w: f64, h: f64) -> Rectangle<i32, Physical> {
    let tl = project_point(ctx, x, y);
    let br = project_point(ctx, x + w, y + h);
    Rectangle::new(tl, Size::from((br.x - tl.x, br.y - tl.y)))
}

/// Apply the fit transform to a native surface element: force a fixed geometry (so the result
/// is independent of the scale the render path queries with), rescale about origin, relocate,
/// crop. The native element must have been created at scale `force_scale`. `rescale` folds in
/// the camera zoom (and fit scale); `reloc` is the camera-projected target; `crop` is the
/// camera-projected clip rect (the slot for content, the output for popups).
fn fit_wrap<R>(
    inner: WaylandSurfaceRenderElement<R>,
    force_scale: f64,
    rescale: Scale<f64>,
    reloc: Point<i32, Physical>,
    crop: Rectangle<i32, Physical>,
    screen: Size<i32, Physical>,
) -> Option<Element<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let forced = ElementWindowSurface { inner, zoom: force_scale };
    let r = RescaleRenderElement::from_element(forced, Point::from((0, 0)), rescale);
    let l = RelocateRenderElement::from_element(r, reloc, Relocate::Relative);
    let c = CropRenderElement::from_element(l, Scale::from(force_scale), crop)?;
    Some(Element::WindowFit(ClampOpaque { inner: c, screen }))
}

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    window: &Window,
    context: &compositor_y5_canvas_draw_context::context::Context,
) -> (Vec<Element<R>>, bool)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    // Windows inside an active capture region must keep rendering (and getting
    // frame callbacks / presentation feedback) even when culled — the capture
    // force-set overrides every visibility gate below.
    let force_capture = window
        .uuid()
        .map(|id| state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE).force_set.contains(&id))
        .unwrap_or(false);

    // Skip drawing windows with their groups collapsed.
    if !force_capture && !window.visible(state) {
        return (vec![], false);
    }
    let bound = compositor_y5_window_interface_draw::bound::calculate(
        state, renderer, size, window, context,
    );

    let ctx = state.viewport_context();
    let output_scale = ctx.scale * state.inner.camera_mut().transform.zoom();
    let zoom = ctx.camera_zoom;
    let cfg = compositor_developer_environment_config_base::base::get();

    let elem_loc = state
        .inner.space_state()
        .state
        .element_location(window)
        .unwrap_or_default();
    let gloc = window.geometry().loc;

    let mut elements: Vec<Element<R>> = Vec::new();
    let root_surface: Option<WlSurface> = window.wl_surface().map(|c| c.into_owned());

    // Decoration borders (computed; pushed after popups so popups sit on top).
    let decoration = compositor_y5_window_decoration_element::scene::scene(
        state, renderer, size, window, context, &bound,
    );

    let Some(root_surface) = root_surface else {
        elements.extend(decoration.into_iter().map(Element::SolidBox));
        return (elements, true);
    };
    let Some(root_view) = with_states(&root_surface, |s| view_of(s)) else {
        elements.extend(decoration.into_iter().map(Element::SolidBox));
        return (elements, true);
    };

    // Compositor-decided slot (authority); None → defer to client (native render, no fit).
    let slot_size = if cfg.window_client_size_fallback {
        window
            .toplevel()
            .and_then(|t| t.with_pending_state(|s| s.size))
            .filter(|s| s.w > 0 && s.h > 0)
            .or_else(|| Some(window.geometry().size))
    } else {
        slot::expected_size(window)
    };

    let render_native = |renderer: &mut R, out: &mut Vec<Element<R>>, surface: &WlSurface, loc: Point<i32, Physical>| {
        let native: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
            renderer,
            surface,
            loc,
            Scale::from(output_scale),
            1.0,
            Kind::Unspecified,
        );
        out.extend(native.into_iter().map(|inner| {
            Element::Window(ClampOpaque {
                inner: ElementWindowSurface { inner, zoom: output_scale },
                screen: size,
            })
        }));
    };

    let Some(slot_size) = slot_size.filter(|s| s.w > 0 && s.h > 0) else {
        // Native fallback: toplevel + popups at the standard render location, no fit.
        let render_at = project_point(ctx, (elem_loc.x - gloc.x) as f64, (elem_loc.y - gloc.y) as f64);
        for (popup, location) in PopupManager::popups_for_surface(&root_surface) {
            let pg = popup.geometry().loc;
            let mut pl = project_point(
                ctx,
                (elem_loc.x + location.x - pg.x) as f64,
                (elem_loc.y + location.y - pg.y) as f64,
            );
            if matches!(&popup, PopupKind::InputMethod(_)) {
                // IME popups render at a CONSTANT readable size (screen scale, no zoom); position
                // still tracks the caret. Clamped to the output since this path has no fit/crop.
                if let Some(sz) = popup_dst(popup.wl_surface()) {
                    let pw = (sz.w as f64 * ctx.scale).round() as i32;
                    let ph = (sz.h as f64 * ctx.scale).round() as i32;
                    pl.x = pl.x.clamp(0, (size.w - pw).max(0));
                    pl.y = pl.y.clamp(0, (size.h - ph).max(0));
                }
                let native: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
                    renderer,
                    popup.wl_surface(),
                    pl,
                    Scale::from(ctx.scale),
                    1.0,
                    Kind::Unspecified,
                );
                elements.extend(native.into_iter().map(|inner| {
                    Element::Window(ClampOpaque {
                        inner: ElementWindowSurface { inner, zoom: ctx.scale },
                        screen: size,
                    })
                }));
                continue;
            }
            render_native(renderer, &mut elements, popup.wl_surface(), pl);
        }
        elements.extend(decoration.into_iter().map(Element::SolidBox));
        render_native(renderer, &mut elements, &root_surface, render_at);
        return (elements, true);
    };

    // ── Fitted path ─────────────────────────────────────────────────────────────────
    // Shared fit decision (margin-fill vs letterbox; see `fit::window_fit`).
    // A resize is in flight → stretch the geometry to fill the slot until the client commits the
    // new size, so the window follows the cursor continuously (identity once it catches up).
    let stretch = slot::resize_stretching(window, window.geometry().size);
    let WindowFit { fit_sx, fit_sy, fit_surf, ref_size, cover } = window_fit(
        elem_loc,
        window.geometry(),
        root_view.dst,
        window.bbox(),
        slot_size,
        cfg.window_subsurface_shrinks,
        stretch,
    );
    let (fit_surf_x, fit_surf_y) = fit_surf;

    let rescale = Scale::from((fit_sx * zoom, fit_sy * zoom));
    let reloc = project_point(ctx, fit_surf_x, fit_surf_y);
    let crop_slot = project_rect(ctx, elem_loc.x as f64, elem_loc.y as f64, slot_size.w as f64, slot_size.h as f64);
    // When rendering a split/floating viewport pane, clamp content + popups to the
    // pane's physical rect so a window near the pane edge can't bleed into the
    // neighbour pane. Full-output render (no render target) → the whole output.
    let pane = state.inner.render_target.map(|rt| {
        Rectangle::new(
            Point::from(((rt.origin_logical.0 * ctx.scale).round() as i32, (rt.origin_logical.1 * ctx.scale).round() as i32)),
            Size::from((rt.size_physical.0.round() as i32, rt.size_physical.1.round() as i32)),
        )
    });
    let crop_output = pane.unwrap_or(Rectangle::new(Point::from((0, 0)), size));
    let crop_slot = match pane {
        Some(p) => crop_slot.intersection(p).unwrap_or_default(),
        None => crop_slot,
    };

    // Popups (front): positioned in the SAME fit frame as the content so they stick to the
    // rendered window content, not the raw slot. A popup's `location` is geometry-relative, but
    // when the client's declared geometry is smaller than what's actually rendered (`ref_size` =
    // the fitted reference: VP DEST's viewport `view.dst`, BUF DELTA's oversized buffer), a
    // geometry-relative anchor would land inside the visible content. So map the popup's
    // geometry-relative offset PROPORTIONALLY onto the visible content by `ref_size / geom`
    // (identity for well-behaved windows). The popup's own size is NOT scaled by this (only by
    // `fit_s` via `fit_wrap`), so a corner-anchored popup lands within ~one popup-size of the
    // corner — accepted tradeoff. Cropped to the **output** so a popup may extend past the
    // window. Popups never resize the toplevel. `hit.rs` mirrors this exact mapping.
    let geom_size = window.geometry().size;
    // margin regime → smithay's standard offset (`gloc + location − pg`); cursor-anchored menus
    // (real apps) land on the cursor. oversized regime → proportional pin to the visible content.
    let psx = if cover || geom_size.w <= 0 { 1.0 } else { ref_size.w as f64 / geom_size.w as f64 };
    let psy = if cover || geom_size.h <= 0 { 1.0 } else { ref_size.h as f64 / geom_size.h as f64 };
    let gbase = if cover { gloc } else { Point::from((0, 0)) };
    for (popup, location) in PopupManager::popups_for_surface(&root_surface) {
        let pg = popup.geometry().loc;
        let off_x = (gbase.x as f64 + (location.x - pg.x) as f64 * psx) * ctx.scale;
        let off_y = (gbase.y as f64 + (location.y - pg.y) as f64 * psy) * ctx.scale;
        if matches!(&popup, PopupKind::InputMethod(_)) {
            // IME candidate popups: the anchor POSITION still tracks the caret through the camera
            // (`reloc + off*rescale`, so it follows pan/zoom), but the SIZE is held constant —
            // rendered at screen scale with `rescale = 1` (no zoom, no window fit) so the list
            // stays a readable size at any zoom, mirroring the screen-space selection UI. Then
            // clamped to the output at that constant size so it never spills off-screen. xdg
            // popups (below) keep scaling with the pannable world.
            let mut ax = reloc.x as f64 + off_x * rescale.x;
            let mut ay = reloc.y as f64 + off_y * rescale.y;
            if let Some(sz) = popup_dst(popup.wl_surface()) {
                let fw = sz.w as f64 * ctx.scale;
                let fh = sz.h as f64 * ctx.scale;
                ax = ax.clamp(0.0, (size.w as f64 - fw).max(0.0));
                ay = ay.clamp(0.0, (size.h as f64 - fh).max(0.0));
            }
            let anchor = Point::from((ax.round() as i32, ay.round() as i32));
            let native: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                Point::from((0, 0)),
                Scale::from(ctx.scale),
                1.0,
                Kind::Unspecified,
            );
            for inner in native {
                if let Some(e) = fit_wrap(inner, ctx.scale, Scale::from((1.0, 1.0)), anchor, crop_output, size) {
                    elements.push(e);
                }
            }
            continue;
        }
        let native: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
            renderer,
            popup.wl_surface(),
            Point::from((off_x.round() as i32, off_y.round() as i32)),
            Scale::from(ctx.scale),
            1.0,
            Kind::Unspecified,
        );
        for inner in native {
            if let Some(e) = fit_wrap(inner, ctx.scale, rescale, reloc, crop_output, size) {
                elements.push(e);
            }
        }
    }

    // Decoration borders.
    elements.extend(decoration.into_iter().map(Element::SolidBox));

    // Toplevel content: native at (0,0) @ ctx.scale, then fitted + cropped to the slot.
    let native: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
        renderer,
        &root_surface,
        Point::from((0, 0)),
        Scale::from(ctx.scale),
        1.0,
        Kind::Unspecified,
    );
    for inner in native {
        if let Some(e) = fit_wrap(inner, ctx.scale, rescale, reloc, crop_slot, size) {
            elements.push(e);
        }
    }

    // Opaque black fill behind the content covering the whole slot. Pushed (so drawn behind the
    // content) whenever the content doesn't fill the slot (letterbox) OR a resize is in flight: a
    // resizing client can commit a blank/no-content frame for a few frames right after acking, and
    // the fill keeps the background from flashing through during that gap (the window shows the
    // fill, not whatever is behind it).
    let fills = ref_size.w as f64 * fit_sx >= slot_size.w as f64 - 1.0
        && ref_size.h as f64 * fit_sy >= slot_size.h as f64 - 1.0;
    if !fills || stretch {
        let black = SolidColorRenderElement::new(
            Id::new(),
            crop_slot,
            CommitCounter::default(),
            [0.0, 0.0, 0.0, 1.0],
            Kind::Unspecified,
        );
        elements.push(Element::SolidBox(black));
    }

    (elements, true)
}
