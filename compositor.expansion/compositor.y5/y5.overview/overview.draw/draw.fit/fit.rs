//! Fit one window's surface tree into a target rect — without the camera.
//!
//! Mirrors `window.draw.frame`'s fit math (`Rescale` → `Relocate` → `Crop`) but
//! targets an arbitrary physical rect instead of the camera-projected world
//! slot, so the overview grid can present a window at its true aspect without
//! remapping it.

use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::utils::{
    CropRenderElement, Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size};
use compositor_y5_window_draw_element::element::{ClampOpaque, Element as WindowElement, ElementWindowSurface};
use compositor_y5_canvas_draw_element::element::Element as CanvasElement;

/// Render `root`'s surface tree and fit it (uniform scale, geometry-top-left
/// aligned to the cell, cropped to it) into `cell` at its true aspect. `geom` is
/// the window geometry (logical); `scale` is the native render scale; `screen`
/// is the output size (for `ClampOpaque`).
pub fn fit_window<R>(
    renderer: &mut R,
    root: &WlSurface,
    geom: Rectangle<i32, Logical>,
    cell: Rectangle<i32, Physical>,
    scale: f64,
    screen: Size<i32, Physical>,
) -> Vec<CanvasElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let geom_h = geom.size.h as f64;
    if geom_h <= 0.0 {
        return Vec::new();
    }
    // Uniform scale mapping geometry height → cell height (width follows at the
    // true aspect, matching the grid cell width).
    let s = cell.size.h as f64 / (geom_h * scale);
    let rescale = Scale::from((s, s));
    // Move the geometry top-left (at gloc*scale*s after rescale) to the cell
    // origin; native is rendered at the origin so this is a relative shift.
    let reloc = Point::<i32, Physical>::from((
        (cell.loc.x as f64 - geom.loc.x as f64 * scale * s).round() as i32,
        (cell.loc.y as f64 - geom.loc.y as f64 * scale * s).round() as i32,
    ));

    let native: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
        renderer,
        root,
        Point::from((0, 0)),
        Scale::from(scale),
        1.0,
        Kind::Unspecified,
    );
    let mut out = Vec::new();
    for inner in native {
        // The overview always suppresses the bundle's claim (its grid sits on a
        // frozen capture, not the live band), so nothing here is bundle-drawn.
        let forced = ElementWindowSurface { inner, zoom: scale, covered: false };
        let r = RescaleRenderElement::from_element(forced, Point::from((0, 0)), rescale);
        let l = RelocateRenderElement::from_element(r, reloc, Relocate::Relative);
        let Some(c) = CropRenderElement::from_element(l, Scale::from(scale), cell) else {
            continue;
        };
        out.push(CanvasElement::Window(WindowElement::WindowFit(ClampOpaque {
            inner: c,
            screen,
            covered: false,
        })));
    }
    out
}
