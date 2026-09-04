//! Fit-to-bounds (letterbox) computation — shared by the window render path
//! (`window.draw.frame::scene`) and the input hit-test path (`surface.interface.base::hit`)
//! so both agree on how a window whose committed content differs from its recognized bounds
//! is placed.
//!
//! ## Why
//!
//! y5 lays out and routes input by a window's **recognized bounds** (`element_geometry` —
//! the xdg geometry the compositor accepts) but a misbehaving client can commit content
//! (`element_bbox`) that overflows, underflows, or is offset from those bounds (oversized
//! buffers, lying `set_window_geometry`, viewport / fractional / integer-scale mismatch,
//! negative subsurface offsets). Rendering the raw `bbox` then spills the window outside its
//! bounds and desyncs the cursor.
//!
//! ## Strategy (product guidance — no stretching)
//!
//! When the rendered content extent does not match the recognized bounds, scale the content
//! **uniformly** (preserving aspect ratio — never distort) to the largest size that fits
//! inside the bounds, centre it, and letterbox the remainder with black. Reduces to the
//! identity transform when content == bounds, so well-behaved windows are untouched.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// How much larger (logical px, per axis) a surface may be than its slot before the excess is
/// treated as oversized **content** (letterbox it) rather than **margin** (shadow / reserved
/// space — fill the slot and crop it). Real CSD apps overshoot by a shadow's worth (tens of px);
/// the stress harness's oversized/viewport cases overshoot by hundreds.
pub const MARGIN_FILL_THRESHOLD: i32 = 250;

/// How far a surface may fall **short** of its slot (logical px, per axis) and still be drawn at
/// 1:1 rather than being magnified to fill it.
///
/// The `cover` regime exists for surfaces that overshoot the slot (CSD shadow — fill and crop the
/// margin). A surface that undershoots has nothing to crop away, so `cover` magnifies it and then
/// shaves the edges, which is strictly worse than showing all of it: the content is resampled off
/// its own pixel grid AND loses its outermost pixels.
///
/// The undershoot is usually not a client bug and cannot be configured away. Cell-quantised
/// clients — terminals such as foot, which round the window down to a whole character grid — can
/// only ever commit a multiple of their cell size, so they land a fraction of a cell short of
/// whatever slot they are given and **stay** there: the compositor re-sends the decided size, the
/// client re-answers with the same quantised one, and `reassert_size_if_diverged` exhausts its
/// nudges and gives up. The magnification is therefore permanent, not a transient during settling.
///
/// Sized to clear a character cell with room to spare while staying far below
/// [`MARGIN_FILL_THRESHOLD`], so a window that is *genuinely* undersized still grows into its slot
/// instead of sitting in a large black frame.
pub const QUANTIZE_SLACK: i32 = 64;

/// The resolved placement of a window's content inside its compositor-decided **slot**, shared
/// by the render path (`window.draw.frame::scene`) and the input path
/// (`surface.interface.base::hit`) so they never disagree.
///
/// `fit_surf` is the world position of the **main-surface origin** (surface-local `(0,0)`); a
/// surface-local point `p` is displayed at world `fit_surf + p * fit_s`. `ref_size` is the
/// reference the popup proportional-pin uses (`ref_size / geometry`).
#[derive(Debug, Clone, Copy)]
pub struct WindowFit {
    /// Per-axis scale (surface-local → world). Equal on both axes for every aspect-preserving
    /// regime; they differ **only** in `stretch` mode (a resize in flight), where the stale
    /// buffer is stretched non-uniformly to exactly fill the slot.
    pub fit_sx: f64,
    pub fit_sy: f64,
    pub fit_surf: (f64, f64),
    pub ref_size: Size<i32, Logical>,
    /// True in the **margin** regime (surface ≈ slot; geometry covers the slot) and in `stretch`
    /// mode. Popups then use smithay's standard offset `geometry().loc + location −
    /// popup.geometry().loc` so cursor-anchored menus land on the cursor. False in the
    /// **oversized** regime, where popups are proportionally pinned to the visible content.
    pub cover: bool,
}

/// Decide how a window's surface tree is fitted into its slot.
///
/// - `elem_loc` — the window's `element_location` (world; the slot origin).
/// - `geom` — `window.geometry()` (the client's declared window: content, excludes CSD shadow).
/// - `view_dst` — the root surface's logical size (`SurfaceView.dst`; reflects viewport / scale).
/// - `slot` — the compositor-decided window size.
///
/// Three regimes:
/// - `subsurface_shrinks` (experimental flag): fit the whole tree (`bbox`), contain/letterbox.
/// - **margin** (`|view_dst − slot| ≤ MARGIN_FILL_THRESHOLD`): the surface is about slot-sized,
///   so the excess is shadow/reserved space — fit the **geometry** and **cover** the slot
///   (fill it, crop the small margin; no letterbox). This is the common real-app path and makes
///   the popup factor `ref_size/geom == 1`, so cursor-anchored menus stay on the cursor. `cover`
///   only ever shrinks: a surface that falls SHORT of the slot by at most [`QUANTIZE_SLACK`] is
///   held at 1:1 and letterboxed instead of being magnified into it.
/// - **oversized** (genuine, large mismatch — viewport/oversized buffer): fit `view_dst` and
///   **contain** (letterbox), so the whole surface stays visible.
///
/// `stretch` overrides all of the above: a resize is in flight and the client's resized buffer
/// hasn't landed yet, so stretch the current `view_dst` buffer **non-uniformly** to exactly fill
/// the slot — the window follows the cursor smoothly with no letterbox flicker; the brief
/// distortion resolves the instant the real buffer arrives.
pub fn window_fit(
    elem_loc: Point<i32, Logical>,
    geom: Rectangle<i32, Logical>,
    view_dst: Size<i32, Logical>,
    slot: Size<i32, Logical>,
    stretch: bool,
) -> WindowFit {
    if stretch {
        // Stretch the **geometry** (the client's content, excluding CSD shadow) to fill the slot
        // exactly — the SAME reference the steady-state `cover` regime uses. Using `view_dst`
        // (content + shadow) would stretch the content smaller, then jump bigger when the resize
        // settles to `cover`. Per-axis so the content tracks the slot's aspect with no letterbox.
        let fit_sx = (slot.w as f64 / geom.size.w.max(1) as f64).max(1e-6);
        let fit_sy = (slot.h as f64 / geom.size.h.max(1) as f64).max(1e-6);
        // Geometry origin (surface-local `geom.loc`) → slot origin; geometry fills the slot.
        return WindowFit {
            fit_sx,
            fit_sy,
            fit_surf: (
                elem_loc.x as f64 - geom.loc.x as f64 * fit_sx,
                elem_loc.y as f64 - geom.loc.y as f64 * fit_sy,
            ),
            ref_size: geom.size,
            cover: true,
        };
    }

    let excess = (view_dst.w - slot.w).abs().max((view_dst.h - slot.h).abs());
    let (ref_size, ref_loc, cover) = if excess <= MARGIN_FILL_THRESHOLD {
        (geom.size, geom.loc, true)
    } else {
        (view_dst, Point::from((0, 0)), false)
    };
    let fit_s = {
        let sw = slot.w as f64 / (ref_size.w.max(1) as f64);
        let sh = slot.h as f64 / (ref_size.h.max(1) as f64);
        let s = if cover { sw.max(sh) } else { sw.min(sh) };
        // Never MAGNIFY to paper over a small shortfall (see `QUANTIZE_SLACK`): a surface that
        // undershoots its slot has nothing to crop, so `cover`'s `max` would only resample it off
        // its own pixel grid and shave the edges. Hold 1:1 and let the caller's letterbox bars —
        // the slot minus the content, which it already paints — take the remainder. A surface that
        // OVERSHOOTS is untouched: `s <= 1.0` there, and cropping the margin is the point.
        let short = (slot.w - ref_size.w).max(slot.h - ref_size.h);
        let s = if s > 1.0 && short <= QUANTIZE_SLACK { 1.0 } else { s };
        if s.is_finite() && s > 0.0 { s } else { 1.0 }
    };
    let fit_surf = (
        elem_loc.x as f64 + (slot.w as f64 - ref_size.w as f64 * fit_s) / 2.0 - ref_loc.x as f64 * fit_s,
        elem_loc.y as f64 + (slot.h as f64 - ref_size.h as f64 * fit_s) / 2.0 - ref_loc.y as f64 * fit_s,
    );
    WindowFit { fit_sx: fit_s, fit_sy: fit_s, fit_surf, ref_size, cover }
}

/// Where a popup's surface ORIGIN sits, in the parent's main-surface-local frame, under
/// this fit — the one mapping the render path and the hit test must agree on.
///
/// `location` is the popup's geometry-relative offset as `PopupManager` reports it, and
/// `popup_geom` its own `geometry().loc` (zero for X11, which declares no window
/// geometry — the location half is `PopupKind::location`'s answer instead).
///
/// Two regimes, and the difference is which frame the fit anchored:
///
/// - **cover** (margin / stretch): the fit anchors the client's GEOMETRY, so a
///   geometry-relative offset needs `geom.loc` added to reach surface-local. This is
///   smithay's own `geometry().loc + location - popup.geometry().loc`, byte for byte, and
///   it is what makes a cursor-anchored menu land on the cursor.
/// - **oversized / subsurface-shrink**: the fit anchors the whole surface, so no
///   `geom.loc` term — and the visible content is a SCALED version of the declared
///   geometry, so the offset is pinned proportionally by `ref_size / geom.size`. Without
///   that, a geometry-relative anchor lands somewhere inside the visible content instead
///   of on it.
///
/// The popup's own SIZE is not scaled here (only by `fit_s` at the call site), so a
/// corner-anchored popup lands within about one popup-size of the corner in the
/// proportional regime — a known, accepted approximation.
///
/// Returned in `f64` because the render path multiplies by the output scale before
/// rounding, while the hit test rounds straight to logical integers; rounding here would
/// cost the render path sub-pixel accuracy.
pub fn popup_offset(
    fit: &WindowFit,
    geom: Rectangle<i32, Logical>,
    location: Point<i32, Logical>,
    popup_geom: Point<i32, Logical>,
) -> Point<f64, Logical> {
    let pin = |reference: i32, declared: i32| match fit.cover || declared <= 0 {
        true => 1.0,
        false => reference as f64 / declared as f64,
    };
    let base = match fit.cover {
        true => geom.loc,
        false => Point::from((0, 0)),
    };
    Point::from((
        base.x as f64 + (location.x - popup_geom.x) as f64 * pin(fit.ref_size.w, geom.size.w),
        base.y as f64 + (location.y - popup_geom.y) as f64 * pin(fit.ref_size.h, geom.size.h),
    ))
}

/// The placement of a window's content inside its recognized bounds.
#[derive(Debug, Clone, Copy)]
pub struct Fit {
    /// Uniform content scale (`1.0` = no fit).
    pub scale: f64,
    /// World top-left of the content extent (`element_bbox.loc`).
    pub content_loc: (f64, f64),
    /// World top-left where the content's top-left is displayed after fitting.
    pub fit_loc: (f64, f64),
    /// World origin to hand to `render_elements` (main-surface origin after fitting). When
    /// the fit is the identity this equals the standard render location
    /// (`element_location − geometry().loc`).
    pub render_origin: (f64, f64),
    /// True when the fit differs from the identity (content != bounds): the caller should
    /// draw the black letterbox, and the cursor mapping is non-trivial.
    pub active: bool,
}

impl Fit {
    /// Compute the fit.
    ///
    /// - `geometry` — recognized bounds in world (`element_geometry`).
    /// - `bbox` — rendered content extent in world (`element_bbox`).
    /// - `render_location` — main-surface origin in world (`element_location − geometry().loc`).
    pub fn compute(
        geometry: Rectangle<i32, Logical>,
        bbox: Rectangle<i32, Logical>,
        render_location: (f64, f64),
    ) -> Fit {
        let b = geometry.to_f64();
        let c = bbox.to_f64();
        let content_loc = (c.loc.x, c.loc.y);

        let (cw, ch) = (c.size.w, c.size.h);
        // Degenerate content (unmapped / zero buffer) or bounds: identity, render at the
        // standard render location.
        if cw < 1.0 || ch < 1.0 || b.size.w < 1.0 || b.size.h < 1.0 {
            return Fit {
                scale: 1.0,
                content_loc,
                fit_loc: content_loc,
                render_origin: render_location,
                active: false,
            };
        }

        // Largest uniform scale that fits the content inside the bounds (aspect preserved).
        let s = (b.size.w / cw).min(b.size.h / ch);
        let s = if s.is_finite() && s > 0.0 { s } else { 1.0 };

        let (fit_w, fit_h) = (cw * s, ch * s);
        let fit_loc = (
            b.loc.x + (b.size.w - fit_w) / 2.0,
            b.loc.y + (b.size.h - fit_h) / 2.0,
        );
        // Where the main-surface origin lands so that content_loc maps to fit_loc at scale s.
        let render_origin = (
            fit_loc.0 + (render_location.0 - content_loc.0) * s,
            fit_loc.1 + (render_location.1 - content_loc.1) * s,
        );

        let active = (s - 1.0).abs() > 1e-3
            || (fit_loc.0 - content_loc.0).abs() > 0.5
            || (fit_loc.1 - content_loc.1).abs() > 0.5;

        Fit {
            scale: s,
            content_loc,
            fit_loc,
            render_origin,
            active,
        }
    }

    /// Fit a `content`-sized region into `slot` (both world), preserving aspect, centered.
    /// `content_loc` is `(0,0)` so [`to_content`] yields a **surface-local** point (relative
    /// to the content's own origin), and [`render_origin`](Fit::render_origin) is where the
    /// content's top-left lands in world. Used by the manual surface-tree render/hit path.
    pub fn for_size(
        slot: Rectangle<i32, Logical>,
        content: smithay::utils::Size<i32, Logical>,
    ) -> Fit {
        let b = slot.to_f64();
        let cw = (content.w.max(1)) as f64;
        let ch = (content.h.max(1)) as f64;
        if b.size.w < 1.0 || b.size.h < 1.0 {
            let loc = (b.loc.x, b.loc.y);
            return Fit { scale: 1.0, content_loc: (0.0, 0.0), fit_loc: loc, render_origin: loc, active: false };
        }
        let s = (b.size.w / cw).min(b.size.h / ch);
        let s = if s.is_finite() && s > 0.0 { s } else { 1.0 };
        let (fit_w, fit_h) = (cw * s, ch * s);
        let fit_loc = (
            b.loc.x + (b.size.w - fit_w) / 2.0,
            b.loc.y + (b.size.h - fit_h) / 2.0,
        );
        let active = (s - 1.0).abs() > 1e-3
            || (fit_w - b.size.w).abs() > 0.5
            || (fit_h - b.size.h).abs() > 0.5;
        Fit { scale: s, content_loc: (0.0, 0.0), fit_loc, render_origin: fit_loc, active }
    }

    /// Map a world point (in the displayed / recognized space, e.g. the cursor) into the
    /// **unscaled content** world coordinate, inverting the fit. Identity when `!active`.
    pub fn to_content(&self, world: (f64, f64)) -> (f64, f64) {
        (
            self.content_loc.0 + (world.0 - self.fit_loc.0) / self.scale,
            self.content_loc.1 + (world.1 - self.fit_loc.1) / self.scale,
        )
    }
}
