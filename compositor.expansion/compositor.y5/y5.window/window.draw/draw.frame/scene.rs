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
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::utils::{RendererSurfaceStateUserData, SurfaceView};
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
use compositor_orchestration_draw_scene_identity::identity::SolidBank;
use compositor_support_smithay_state_window_shell::shell;

/// Per-window slot bank for the letterbox bars. A NEWTYPE, not a bare
/// `SolidBank`: `UserDataMap` is keyed by type, and the decoration crate stores
/// its own bank on the same window — unwrapped, the two would share slots.
/// Named `...Bars` because `scene` locally aliases `LetterboxMode as Letterbox`.
#[derive(Default)]
struct LetterboxBars(SolidBank);

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

/// How far off a pane a window may sit and still count as being on it for
/// `xdg_toplevel.suspended` — world-logical units, scaled to physical the same way
/// [`DECORATION_MARGIN`] is.
///
/// Nothing renders in this band; a window past the frustum draws no pixels at all,
/// decorations and popups included, because the cull returns before either. It exists
/// only so a window parked just past the edge is not told to stop repainting when the
/// smallest pan brings it back — the same bargain `FRACTIONAL_GRACE_RANGE` makes for
/// scale, at a radius sized for a state whose recovery costs a client a full repaint
/// rather than a buffer re-scale.
///
/// Unconditional, unlike `FRACTIONAL_GRACE_RANGE`, which only applies under the "full"
/// invisible-window strategy: a scale published early is an optimisation, and telling a
/// window it is not being repainted when it is about to be is a correctness question.
const SUSPEND_GRACE: f64 = 512.0;

/// Where a window's slot lands on the pane being drawn.
enum Placed {
    /// On screen here. Carries the rect the occlusion test uses: the projected
    /// slot inflated by [`DECORATION_MARGIN`] and re-clipped to the pane (the
    /// re-clip is what still lets a pane-filling window be occluded at all).
    On(Rectangle<i32, Physical>),
    /// Degenerate size — nothing committed yet. Draw it; there is no rect to
    /// cull against or to occlude with.
    Unsized,
    /// Entirely off this pane. `true` when it is nonetheless inside
    /// [`SUSPEND_GRACE`] of it, which draws nothing but holds off `suspended`.
    Off { near: bool },
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
    slot::expected_size(window)
}

fn placed_on(state: &mut Loop, window: &Window, size: Size<i32, Physical>) -> Placed {
    let Some(loc) = state.inner.space_state().state.element_location(window) else {
        return Placed::Off { near: false };
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
        // Same rect against the pane grown by the suspend grace band. Only the
        // `suspended` state reads this; nothing here draws.
        let grace = (SUSPEND_GRACE * ctx.scale).ceil() as i32;
        let near = rect.overlaps(Rectangle::new(
            Point::from((pane.loc.x - grace, pane.loc.y - grace)),
            Size::from((pane.size.w + grace * 2, pane.size.h + grace * 2)),
        ));
        return Placed::Off { near };
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
    bundle_owned: bool,
    rescale: Scale<f64>,
    reloc: Point<i32, Physical>,
    crop: Rectangle<i32, Physical>,
    screen: Size<i32, Physical>,
) -> Option<Element<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let forced = ElementWindowSurface { inner, zoom: force_scale, bundle_owned };
    let r = RescaleRenderElement::from_element(forced, Point::from((0, 0)), rescale);
    let l = RelocateRenderElement::from_element(r, reloc, Relocate::Relative);
    let c = CropRenderElement::from_element(l, Scale::from(force_scale), crop)?;
    Some(Element::WindowFit(ClampOpaque { inner: c, screen, bundle_owned }))
}

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    window: &Window,
    context: &compositor_y5_canvas_draw_context::context::Context,
    occluders: &Occluders,
    // Whether the drawn world's bundle composites windows itself — resolved ONCE
    // per frame by the caller, not per window. See `ClampOpaque::bundle_owned`;
    // it is NOT the occlusion sense of "covered" used throughout this file.
    bundle_owned: bool,
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
    let mut drawn = Drawn { on_pane: true, visible: true, on_pane_awake: true, opaque: Vec::new() };
    if !force_capture {
        match placed_on(state, window, size) {
            // Frustum cull: a window whose slot projects outside the pane being
            // drawn contributes nothing — skip its whole scene (surface-tree
            // walk, decorations, fit) and keep it out of BOTH sets.
            //
            // `near` is the one thing that survives the cull: it draws nothing, it
            // just stops the window being told to suspend while it sits a nudge off
            // the edge.
            Placed::Off { near } => {
                return (vec![], Drawn { on_pane_awake: near, ..Drawn::default() });
            }
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
                    // The one exit where the flags disagree — the neighbours above
                    // return `Drawn::default()`, all false.
                    //
                    // `on_pane_awake` stays TRUE, which is where this parts company with the
                    // protocol's own worked example for `suspended`. Occlusion here is
                    // not a mode the user entered: the occluder is a sibling window on
                    // a pannable canvas, and it moves, closes or is scrolled off in one
                    // frame with nothing to announce it. Telling the covered window to
                    // stop repainting buys one window's frames and pays for them with a
                    // stale frame on every reveal, since it has to be configured, then
                    // render, then commit before it can show anything.
                    return (
                        vec![],
                        Drawn { on_pane: true, visible: false, on_pane_awake: true, opaque: Vec::new() },
                    );
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
                inner: ElementWindowSurface { inner, zoom: output_scale, bundle_owned },
                screen: size,
                bundle_owned,
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
                        inner: ElementWindowSurface { inner, zoom: ctx.scale, bundle_owned },
                        screen: size,
                        bundle_owned,
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
    let fit = window_fit(
        elem_loc,
        window.geometry(),
        root_view.dst,
        slot_size,
        stretch,
    );
    let WindowFit { fit_sx, fit_sy, fit_surf, ref_size, cover: _ } = fit;
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
    // rendered window content, not the raw slot. The mapping itself lives in
    // `fit::popup_offset` — ONE definition, because `hit.rs` must resolve a pointer through
    // exactly the same frame or a click lands where the menu is not drawn. Cropped to the
    // **output** so a popup may extend past the window; popups never resize the toplevel.
    let geom_rect = window.geometry();
    for (popup, location) in PopupManager::popups_for_surface(&root_surface) {
        let off = compositor_y5_camera_transform_translate::fit::popup_offset(
            &fit,
            geom_rect,
            location,
            popup.geometry().loc,
        );
        let off_x = off.x * ctx.scale;
        let off_y = off.y * ctx.scale;
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
                if let Some(e) = fit_wrap(inner, ctx.scale, bundle_owned, Scale::from((1.0, 1.0)), anchor, crop_output, size) {
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
            if let Some(e) = fit_wrap(inner, ctx.scale, bundle_owned, rescale, reloc, crop_output, size) {
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
        if let Some(e) = fit_wrap(inner, ctx.scale, bundle_owned, rescale, reloc, crop_slot, size) {
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

    // Opaque black behind the content: the letterbox BARS, which are the slot
    // minus the content. Nothing is drawn under content that is about to cover
    // it, so a translucent client is never silently backed with black.
    // `subtract_rect` returns nothing when the fit covers the slot, so it also
    // replaces the "does it fill?" test that used to gate this.
    //
    // A resize in flight is the same expression, not a special case. `stretch`
    // fits the geometry to exactly fill the slot, so the remainder is empty and
    // no black is painted — which is the point: the full-slot backstop this used
    // to paint showed through every translucent window for the whole drag. It
    // still covers the slot when the geometry is degenerate (nothing committed
    // yet), which is the blank-commit gap the backstop was for.
    //
    // The bundle's `letterbox` policy is INERT: every case takes the bars arm.
    // The manifest field still parses and `policy` is still resolved per world,
    // so re-enabling suppression is uncommenting the arms below. It is off
    // because both suppressing modes existed to escape the full-slot backstop,
    // and with that gone a bundle asking for them gets a worse result than the
    // shared arm already gives it.
    use compositor_pipeline_bundle_manifest_base::manifest::LetterboxMode as Letterbox;
    // THIS world's bundle, not a process-global copy of the last one selected.
    let target = state.inner.worlds.spawn_target();
    let policy: Letterbox = state
        .inner
        .worlds
        .get(target)
        .storage()
        .try_get(&compositor_background_two_storage_base::base::BG_TWO)
        .map(|t| t.letterbox())
        .unwrap_or_default();
    let bars: Vec<Rectangle<i32, Physical>> = match (policy, stretch) {
        // (Letterbox::Always, _) => Vec::new(),
        // (Letterbox::OnResize, true) => Vec::new(),
        // (_, true) => vec![crop_slot],   // the full-slot resize backstop
        _ => crop_slot.subtract_rect(content),
    };
    // Per-window bank (see `scene.identity`), separate from the decoration crate's
    // — distinct types in the window's user data, so borders and bars never collide.
    // `subtract_rect` returns a stable order for a stable fit, so slot `i` keeps
    // meaning the same bar; a fit change reshuffles at most a few, which costs one
    // frame of extra damage rather than every frame of it.
    let letterbox = window.user_data().get_or_insert(LetterboxBars::default);
    for (slot, rect) in bars.iter().enumerate() {
        elements.push(Element::SolidBox(letterbox.0.solid(slot, *rect, [0.0, 0.0, 0.0, 1.0])));
    }

    // Deposit exactly what is opaque — no slack, because the bars are literally
    // the rects just painted at alpha 1.0. The content region joins them only
    // when the client's buffer has no alpha; the two then tile `crop_slot`, so an
    // opaque client in a letterbox still hides the whole slot. A translucent one
    // now hides only the bars, which is the truth and was not expressible while
    // this was a single rect.
    // A drawable the BUNDLE composites is not an occluder, whatever its pixels
    // are. `Own::covers` states both halves — the engine must "neither blit it nor
    // treat it as covering what is behind it" — and only the first half was
    // implemented, in `ClampOpaque::opaque_regions`. This is the second.
    //
    // Depositing anyway culled the window BEHIND one the bundle had moved: the cull
    // returns before the scene is built, so that window produced no element, never
    // reached the world set, and the bundle could not draw it either. It came back
    // only where some other window happened to damage the same pixels.
    //
    // Losing the deposit costs at most a redundant draw (see the module docs); the
    // bars are still painted, they simply stop culling.
    drawn.opaque = if bundle_owned { Vec::new() } else { bars };
    if !bundle_owned
        && surface_opaque(&root_surface)
        && let Some(opaque_content) = content.intersection(crop_slot)
    {
        drawn.opaque.push(opaque_content);
    }

    (elements, drawn)
}
