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
use compositor_y5_window_draw_occlude::occlude::{Drawn, Occluders};
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

/// The pane being drawn (`render_target`) in output-physical space; the whole
/// output when unset (full-output render).
fn pane_rect(state: &Loop, ctx: XformCtx, size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    state
        .inner
        .render_target
        .map(|rt| {
            Rectangle::new(
                Point::from((
                    (rt.origin_logical.0 * ctx.scale).round() as i32,
                    (rt.origin_logical.1 * ctx.scale).round() as i32,
                )),
                Size::from((rt.size_physical.0.round() as i32, rt.size_physical.1.round() as i32)),
            )
        })
        .unwrap_or(Rectangle::new(Point::from((0, 0)), size))
}

/// Widest decoration border (`window.decoration.element`: 12 logical px on the
/// primary selection) plus the 1px outset it is drawn at. Decorations frame the
/// slot from OUTSIDE it, so the occlusion test runs on the slot inflated by this
/// — a covered slot whose frame still shows is not a window we may cull.
const DECORATION_MARGIN: f64 = 13.0;

/// Where a window's slot lands on the pane being drawn.
enum Placed {
    /// On screen here. Carries the rect the occlusion test uses: the projected
    /// slot inflated by [`DECORATION_MARGIN`] and re-clipped to the pane (the
    /// re-clip is what still lets a pane-filling window be occluded at all).
    On(Rectangle<i32, Physical>),
    /// Degenerate size — nothing committed yet. Draw it; there is no rect to
    /// cull against or to occlude with.
    Unsized,
    /// Entirely off this pane.
    Off,
}

/// The compositor-decided slot — the SAME derivation the fit and `crop_slot`
/// below use. The culls have to agree with it exactly: the whole reason a
/// covered window may be skipped is that the slot bounds everything it can ever
/// draw, and a rect derived any other way does not carry that guarantee.
///
/// `None` means no slot has been decided, which sends the fit down the native
/// fallback — content rendered at the client's own location with no crop. So the
/// caller treats it as `Unsized` and culls nothing: there is no bound to rely on.
fn slot_size_of(window: &Window) -> Option<Size<i32, Logical>> {
    if compositor_model_environment_config_base::base::get().window_client_size_fallback {
        window
            .toplevel()
            .and_then(|t| t.with_pending_state(|s| s.size))
            .filter(|s| s.w > 0 && s.h > 0)
            .or_else(|| Some(window.geometry().size))
    } else {
        slot::expected_size(window)
    }
}

fn placed_on(state: &mut Loop, window: &Window, size: Size<i32, Physical>) -> Placed {
    let Some(loc) = state.inner.space_state().state.element_location(window) else {
        return Placed::Off;
    };
    let Some(sz) = slot_size_of(window) else {
        return Placed::Unsized;
    };
    if sz.w <= 0 || sz.h <= 0 {
        return Placed::Unsized;
    }
    let ctx = state.viewport_context();
    // Mirrors `crop_slot`'s projection below, so cull and crop agree.
    let rect = project_rect(ctx, loc.x as f64, loc.y as f64, sz.w as f64, sz.h as f64);
    let pane = pane_rect(state, ctx, size);
    if !rect.overlaps(pane) {
        return Placed::Off;
    }
    let pad = (DECORATION_MARGIN * ctx.scale).ceil() as i32;
    let grown = Rectangle::new(
        Point::from((rect.loc.x - pad, rect.loc.y - pad)),
        Size::from((rect.size.w + pad * 2, rect.size.h + pad * 2)),
    );
    Placed::On(grown.intersection(pane).unwrap_or(rect))
}

fn has_popup(window: &Window) -> bool {
    window
        .wl_surface()
        .is_some_and(|s| PopupManager::popups_for_surface(s.as_ref()).next().is_some())
}

/// True when the root surface is opaque over its whole `dst` — an alpha-free
/// buffer, or a client-declared opaque region that covers it. Subsurfaces are
/// not walked: they can only ADD opacity, so ignoring them errs toward drawing.
fn surface_opaque(surface: &WlSurface) -> bool {
    with_states(surface, |s| {
        let Some(view) = view_of(s) else { return false };
        let Some(m) = s.data_map.get::<RendererSurfaceStateUserData>() else { return false };
        let Ok(g) = m.lock() else { return false };
        let dst = Rectangle::from_size(view.dst);
        g.opaque_regions().is_some_and(|r| r.iter().any(|o| o.contains_rect(dst)))
    })
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
    occluders: &Occluders,
) -> (Vec<Element<R>>, Drawn)
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
        return (vec![], Drawn::default());
    }
    let mut drawn = Drawn { on_pane: true, visible: true, opaque: Vec::new() };
    if !force_capture {
        match placed_on(state, window, size) {
            // Frustum cull: a window whose slot projects outside the pane being
            // drawn contributes nothing — skip its whole scene (surface-tree
            // walk, decorations, fit) and keep it out of BOTH sets.
            Placed::Off => return (vec![], Drawn::default()),
            Placed::Unsized => {}
            // Occlusion cull: fully covered by opaque windows already drawn in
            // front of it. `on_pane` stays set — the window is still on screen,
            // one move away from being revealed with no pan to hide a
            // fractional-scale republish behind, so it keeps its real scale.
            //
            // The SLOT is the right thing to test, and it is sufficient. Content
            // crops to it, the letterbox fill is exactly it, and the slot is the
            // compositor's decision — so a client that answers a configure with
            // a corrected buffer lands inside the same rect. Whether bars are
            // showing changes nothing about what is covered.
            //
            // The two things that do escape the slot are handled: decorations
            // frame it from outside, which is what `DECORATION_MARGIN` inflates
            // for, and popups crop to the OUTPUT, so a window holding one is
            // exempt outright.
            Placed::On(rect) => {
                if occluders.hidden(rect) && !has_popup(window) {
                    // The one exit where the two flags disagree — the neighbours
                    // above return `Drawn::default()`, both false.
                    return (vec![], Drawn { on_pane: true, visible: false, opaque: Vec::new() });
                }
            }
        }
    }
    let bound = compositor_y5_window_interface_draw::bound::calculate(
        state, renderer, size, window, context,
    );

    let ctx = state.viewport_context();
    let output_scale = ctx.scale * state.inner.camera_mut().transform.zoom();
    let zoom = ctx.camera_zoom;
    let cfg = compositor_model_environment_config_base::base::get();

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
        return (elements, drawn);
    };
    let Some(root_view) = with_states(&root_surface, |s| view_of(s)) else {
        elements.extend(decoration.into_iter().map(Element::SolidBox));
        return (elements, drawn);
    };

    // Compositor-decided slot (authority); None → defer to client (native render, no fit).
    let slot_size = slot_size_of(window);

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
        return (elements, drawn);
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
    let crop_output = pane_rect(state, ctx, size);
    let crop_slot = crop_slot.intersection(crop_output).unwrap_or_default();

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

    // The content's own rect, projected by its CORNERS exactly like `crop_slot`
    // so both land on the same lattice. That is what makes the subtraction below
    // exact: when the fit covers the slot the two rects are equal and the
    // remainder is empty, with no epsilon anywhere. The `- ref_loc * fit_s` inside
    // `fit_surf` cancels against `ref_loc`, so this is the same expression in
    // every fit regime (see `fit::window_fit`); `hit.rs` derives it identically.
    let content = project_rect(
        ctx,
        elem_loc.x as f64 + (slot_size.w as f64 - ref_size.w as f64 * fit_sx) / 2.0,
        elem_loc.y as f64 + (slot_size.h as f64 - ref_size.h as f64 * fit_sy) / 2.0,
        ref_size.w as f64 * fit_sx,
        ref_size.h as f64 * fit_sy,
    );

    // Opaque black behind the content. A resize in flight keeps the FULL-slot
    // backstop: the client can commit a blank/no-content frame for a few frames
    // right after acking, and without something opaque under the content the
    // background flashes through that gap.
    //
    // Otherwise only the letterbox BARS are painted — the slot minus the content.
    // Nothing is drawn under content that is about to cover it, and a translucent
    // client stops being silently backed with black. `subtract_rect` returns
    // nothing at all when the fit covers the slot, so it also replaces the
    // "does it fill?" test that used to gate this.
    let bars: Vec<Rectangle<i32, Physical>> =
        if stretch { vec![crop_slot] } else { crop_slot.subtract_rect(content) };
    for rect in &bars {
        elements.push(Element::SolidBox(SolidColorRenderElement::new(
            Id::new(),
            *rect,
            CommitCounter::default(),
            [0.0, 0.0, 0.0, 1.0],
            Kind::Unspecified,
        )));
    }

    // Deposit exactly what is opaque — no slack, because the bars are literally
    // the rects just painted at alpha 1.0. The content region joins them only
    // when the client's buffer has no alpha; the two then tile `crop_slot`, so an
    // opaque client in a letterbox still hides the whole slot. A translucent one
    // now hides only the bars, which is the truth and was not expressible while
    // this was a single rect.
    drawn.opaque = bars;
    // `subsurface_shrinks` fits the whole TREE (`bbox`), so the content rect can
    // reach past the root surface and `surface_opaque` would not be speaking for
    // all of it. Only the bars are claimed under that flag.
    if !cfg.window_subsurface_shrinks
        && surface_opaque(&root_surface)
        && let Some(covered) = content.intersection(crop_slot)
    {
        drawn.opaque.push(covered);
    }

    (elements, drawn)
}
