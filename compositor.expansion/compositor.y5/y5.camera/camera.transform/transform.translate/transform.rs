//! `Transform`: a coordinate carrier with implicit conversion to and
//! from every smithay shape.
//!
//! ## What this does
//!
//! Stores a `(position, size)` pair in **y5-world** (logical units,
//! centre-anchored — `(0, 0)` is the centre of the panel). Provides
//! `From<(value, Context)>` impls for every natural smithay source
//! and every raw tuple, plus `From<Transform>` impls for every
//! natural smithay sink and every raw tuple.
//!
//! At the call site, construction is `(value, ctx).into()`. After
//! that, every onward conversion is plain `.into()` driven by the
//! destination type.
//!
//! ## Semantic
//!
//! - **Input** (`(value, ctx).into()`) interprets smithay's `Logical`
//!   marker as "y5-world coordinates" — the same numbers smithay's
//!   `Space` stores when you `map_element` and gets back from
//!   `element_bbox`. Camera-independent.
//!
//! - **Output** (`.into()`) projects through the camera (pan, zoom)
//!   and the output's scale, depending on the destination type:
//!     - `Rectangle<_, Logical>` → top-left anchored, camera+zoom
//!       applied, logical pixels.
//!     - `Rectangle<_, Physical>` → same, then × scale.
//!     - `Point<_, _>` → top-left of the projected rect.
//!     - `Size<_, _>` → camera zoom applied to size; no anchor.
//!     - Raw `(f64, f64, f64, f64)` → logical, camera applied.
//!
//! - For storing a position back into smithay's Space (e.g.
//!   `map_element`), do **not** route through `Transform` — pass the
//!   world coord directly as `Point::from((x as i32, y as i32))`,
//!   typed as `Point<i32, Logical>` because smithay's API demands
//!   it. Transform's projection would apply the camera, which is
//!   wrong for storage.
//!
//! ## Usage
//!
//! ```ignore
//! let ctx = state.size_ctx_all();
//!
//! // Construct (one of):
//! let t: Transform = (window_bbox, ctx).into();        // smithay logical rect
//! let t: Transform = (cursor_pos, ctx).into();         // smithay logical point
//! let t: Transform = ((wx, wy, w, h), ctx).into();     // raw world rect
//!
//! // Extract (the destination type drives the math):
//! let render_at: Point<i32, Physical> = t.into();
//! let projected: Rectangle<i32, Logical> = t.into();
//! let size: Size<f64, Physical> = t.into();
//! let (x, y, w, h): (i32, i32, i32, i32) = t.into();
//! ```

use smithay::utils as su;

// ─── Context ───────────────────────────────────────────────────────

/// Per-frame conversion snapshot. Construct once per frame from your
/// state; pass into every `(value, ctx).into()` call.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// Camera position in y5-world (logical units).
    pub camera_pos: (f64, f64),
    /// Camera zoom (1.0 = no zoom).
    pub camera_zoom: f64,
    /// The render **region** size in **physical** pixels. For a full-output view
    /// this is `output.current_mode().unwrap().size`; for a split/floating
    /// viewport it is that pane's physical rect size. Content is centred within
    /// this region (see `screen_half_logical`).
    pub screen_size_physical: (f64, f64),
    /// Top-left of the render region in **logical** screen pixels. `(0, 0)` for a
    /// full-output view; the pane's logical origin when rendering into a
    /// sub-region, so the projection lands inside that pane instead of the output.
    pub region_origin: (f64, f64),
    /// Fractional scale (`output.current_scale().fractional_scale()`).
    pub scale: f64,
}

impl Context {
    /// Full-output context (region origin `(0, 0)`, region = whole output).
    pub fn new(
        camera_pos: (f64, f64),
        camera_zoom: f64,
        screen_size_physical: (f64, f64),
        scale: f64,
    ) -> Self {
        Self {
            camera_pos,
            camera_zoom,
            screen_size_physical,
            region_origin: (0.0, 0.0),
            scale,
        }
    }

    /// Sub-region context: render `region_size_physical` at logical
    /// `region_origin`, projecting `camera` to the region's centre. Used to draw
    /// one world through several viewports (split / floating panes).
    pub fn new_region(
        camera_pos: (f64, f64),
        camera_zoom: f64,
        region_origin: (f64, f64),
        region_size_physical: (f64, f64),
        scale: f64,
    ) -> Self {
        Self {
            camera_pos,
            camera_zoom,
            screen_size_physical: region_size_physical,
            region_origin,
            scale,
        }
    }

    /// Logical screen size, derived from physical / scale.
    #[inline]
    pub fn screen_size_logical(&self) -> (f64, f64) {
        (
            self.screen_size_physical.0 / self.scale,
            self.screen_size_physical.1 / self.scale,
        )
    }

    #[inline]
    fn screen_half_logical(&self) -> (f64, f64) {
        let (w, h) = self.screen_size_logical();
        (w / 2.0, h / 2.0)
    }
}

// ─── Transform ─────────────────────────────────────────────────────

/// A rectangle (position + size) in y5-world logical coordinates,
/// plus a context snapshot. Construct via `(value, ctx).into()`,
/// extract via plain `.into()`.
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    /// World position (top-left corner of the rect), logical units.
    pos: (f64, f64),
    /// Size, logical units. Anchor-independent.
    size: (f64, f64),
    ctx: Context,
}

impl Transform {
    pub fn context(&self) -> Context {
        self.ctx
    }

    /// y5-world top-left.
    pub fn world_pos(&self) -> (f64, f64) {
        self.pos
    }

    /// Size in y5-world logical (no camera applied).
    pub fn world_size(&self) -> (f64, f64) {
        self.size
    }

    /// Raw y5-world position as a smithay `Point<i32, Logical>`,
    /// **without** applying the camera projection.
    ///
    /// Use this when storing into smithay's `Space` (e.g.
    /// `map_element`): we want the camera-independent world value
    /// stored, because the camera is applied at render time, not at
    /// storage time. The default `Into<Point<i32, Logical>>`
    /// projects through the camera and is wrong for storage.
    pub fn into_storage_point(self) -> su::Point<i32, su::Logical> {
        su::Point::from((self.pos.0.round() as i32, self.pos.1.round() as i32))
    }

    /// Same as `into_storage_point` but f64.
    pub fn into_storage_point_f64(self) -> su::Point<f64, su::Logical> {
        su::Point::from((self.pos.0, self.pos.1))
    }

    /// Raw y5-world rect as a smithay `Rectangle<i32, Logical>`,
    /// **without** camera projection. Mirror of `into_storage_point`
    /// for full rects.
    pub fn into_storage_rect(self) -> su::Rectangle<i32, su::Logical> {
        su::Rectangle::new(
            su::Point::from((self.pos.0.round() as i32, self.pos.1.round() as i32)),
            su::Size::from((self.size.0.round() as i32, self.size.1.round() as i32)),
        )
    }

    /// Raw y5-world × scale as a smithay `Rectangle<i32, Physical>`,
    /// **without** applying the camera projection. Use this for
    /// downstream consumers (like iced World-space) that take
    /// physical-typed values but apply their own camera math.
    pub fn into_storage_rect_physical(self) -> su::Rectangle<i32, su::Physical> {
        let s = self.ctx.scale;
        su::Rectangle::new(
            su::Point::from((
                (self.pos.0 * s).round() as i32,
                (self.pos.1 * s).round() as i32,
            )),
            su::Size::from((
                (self.size.0 * s).round() as i32,
                (self.size.1 * s).round() as i32,
            )),
        )
    }
}

// ─── Projection (private) ──────────────────────────────────────────

impl Transform {
    /// Project world rect → screen-logical rect (top-left anchored,
    /// camera applied).
    fn to_logical(&self) -> (f64, f64, f64, f64) {
        let zoom = self.ctx.camera_zoom;
        let (cam_x, cam_y) = self.ctx.camera_pos;
        let (half_w, half_h) = self.ctx.screen_half_logical();
        let (ox, oy) = self.ctx.region_origin;
        // Position: subtract camera, scale by zoom, re-anchor to the region centre
        // (region origin + half the region), so a sub-region pane lands in place.
        let x = (self.pos.0 - cam_x) * zoom + half_w + ox;
        let y = (self.pos.1 - cam_y) * zoom + half_h + oy;
        // Size: only zoom (no pan, no anchor).
        let w = self.size.0 * zoom;
        let h = self.size.1 * zoom;
        (x, y, w, h)
    }

    fn to_physical(&self) -> (f64, f64, f64, f64) {
        let (x, y, w, h) = self.to_logical();
        let s = self.ctx.scale;
        (x * s, y * s, w * s, h * s)
    }
}

// ─── Construction: From<(value, Context)> for Transform ────────────
//
// All smithay logical inputs are interpreted as y5-world values (the
// numbers smithay's Space stores).
//
// All smithay physical inputs are interpreted as already-projected
// screen positions, so we divide by scale and reverse the camera
// projection to recover y5-world. This makes round-tripping work
// when the same Transform is built from physical and then extracted
// to physical again.

impl From<(su::Rectangle<i32, su::Logical>, Context)> for Transform {
    fn from((r, ctx): (su::Rectangle<i32, su::Logical>, Context)) -> Self {
        Self {
            pos: (r.loc.x as f64, r.loc.y as f64),
            size: (r.size.w as f64, r.size.h as f64),
            ctx,
        }
    }
}

impl From<(su::Rectangle<f64, su::Logical>, Context)> for Transform {
    fn from((r, ctx): (su::Rectangle<f64, su::Logical>, Context)) -> Self {
        Self {
            pos: (r.loc.x, r.loc.y),
            size: (r.size.w, r.size.h),
            ctx,
        }
    }
}

impl From<(su::Point<i32, su::Logical>, Context)> for Transform {
    fn from((p, ctx): (su::Point<i32, su::Logical>, Context)) -> Self {
        Self {
            pos: (p.x as f64, p.y as f64),
            size: (0.0, 0.0),
            ctx,
        }
    }
}

impl From<(su::Point<f64, su::Logical>, Context)> for Transform {
    fn from((p, ctx): (su::Point<f64, su::Logical>, Context)) -> Self {
        Self {
            pos: (p.x, p.y),
            size: (0.0, 0.0),
            ctx,
        }
    }
}

impl From<(su::Size<i32, su::Logical>, Context)> for Transform {
    fn from((s, ctx): (su::Size<i32, su::Logical>, Context)) -> Self {
        Self {
            pos: (0.0, 0.0),
            size: (s.w as f64, s.h as f64),
            ctx,
        }
    }
}

impl From<(su::Size<f64, su::Logical>, Context)> for Transform {
    fn from((s, ctx): (su::Size<f64, su::Logical>, Context)) -> Self {
        Self {
            pos: (0.0, 0.0),
            size: (s.w, s.h),
            ctx,
        }
    }
}

// Physical input → reverse-project to world. Round-trip:
//   t = (phys_rect, ctx).into(); let back: Rectangle<_, Physical> = t.into();
// should give back == phys_rect.

fn physical_in_to_world(
    px: f64,
    py: f64,
    pw: f64,
    ph: f64,
    ctx: Context,
) -> ((f64, f64), (f64, f64)) {
    // Physical → screen logical.
    let lx = px / ctx.scale;
    let ly = py / ctx.scale;
    let lw = pw / ctx.scale;
    let lh = ph / ctx.scale;
    // Reverse the camera projection. Project formula is:
    //   x = (world_x - cam_x) * zoom + half_w
    // Solving for world_x:
    //   world_x = (x - half_w) / zoom + cam_x
    let zoom = ctx.camera_zoom;
    let (cam_x, cam_y) = ctx.camera_pos;
    let (half_w, half_h) = ctx.screen_half_logical();
    let (ox, oy) = ctx.region_origin;
    let world_x = (lx - half_w - ox) / zoom + cam_x;
    let world_y = (ly - half_h - oy) / zoom + cam_y;
    let world_w = lw / zoom;
    let world_h = lh / zoom;
    ((world_x, world_y), (world_w, world_h))
}

impl From<(su::Rectangle<i32, su::Physical>, Context)> for Transform {
    fn from((r, ctx): (su::Rectangle<i32, su::Physical>, Context)) -> Self {
        let (pos, size) = physical_in_to_world(
            r.loc.x as f64,
            r.loc.y as f64,
            r.size.w as f64,
            r.size.h as f64,
            ctx,
        );
        Self { pos, size, ctx }
    }
}

impl From<(su::Rectangle<f64, su::Physical>, Context)> for Transform {
    fn from((r, ctx): (su::Rectangle<f64, su::Physical>, Context)) -> Self {
        let (pos, size) = physical_in_to_world(r.loc.x, r.loc.y, r.size.w, r.size.h, ctx);
        Self { pos, size, ctx }
    }
}

impl From<(su::Point<i32, su::Physical>, Context)> for Transform {
    fn from((p, ctx): (su::Point<i32, su::Physical>, Context)) -> Self {
        let (pos, _) = physical_in_to_world(p.x as f64, p.y as f64, 0.0, 0.0, ctx);
        Self {
            pos,
            size: (0.0, 0.0),
            ctx,
        }
    }
}

impl From<(su::Point<f64, su::Physical>, Context)> for Transform {
    fn from((p, ctx): (su::Point<f64, su::Physical>, Context)) -> Self {
        let (pos, _) = physical_in_to_world(p.x, p.y, 0.0, 0.0, ctx);
        Self {
            pos,
            size: (0.0, 0.0),
            ctx,
        }
    }
}

impl From<(su::Size<i32, su::Physical>, Context)> for Transform {
    fn from((s, ctx): (su::Size<i32, su::Physical>, Context)) -> Self {
        // Size is camera-independent — only divide by scale × zoom.
        Self {
            pos: (0.0, 0.0),
            size: (
                s.w as f64 / ctx.scale / ctx.camera_zoom,
                s.h as f64 / ctx.scale / ctx.camera_zoom,
            ),
            ctx,
        }
    }
}

impl From<(su::Size<f64, su::Physical>, Context)> for Transform {
    fn from((s, ctx): (su::Size<f64, su::Physical>, Context)) -> Self {
        Self {
            pos: (0.0, 0.0),
            size: (
                s.w / ctx.scale / ctx.camera_zoom,
                s.h / ctx.scale / ctx.camera_zoom,
            ),
            ctx,
        }
    }
}

// Raw tuples → world.

impl From<((f64, f64, f64, f64), Context)> for Transform {
    fn from(((x, y, w, h), ctx): ((f64, f64, f64, f64), Context)) -> Self {
        Self {
            pos: (x, y),
            size: (w, h),
            ctx,
        }
    }
}

impl From<((i32, i32, i32, i32), Context)> for Transform {
    fn from(((x, y, w, h), ctx): ((i32, i32, i32, i32), Context)) -> Self {
        Self {
            pos: (x as f64, y as f64),
            size: (w as f64, h as f64),
            ctx,
        }
    }
}

impl From<((f64, f64), Context)> for Transform {
    fn from(((x, y), ctx): ((f64, f64), Context)) -> Self {
        Self {
            pos: (x, y),
            size: (0.0, 0.0),
            ctx,
        }
    }
}

impl From<((i32, i32), Context)> for Transform {
    fn from(((x, y), ctx): ((i32, i32), Context)) -> Self {
        Self {
            pos: (x as f64, y as f64),
            size: (0.0, 0.0),
            ctx,
        }
    }
}

// ─── Extraction: From<Transform> for X ─────────────────────────────

impl From<Transform> for su::Rectangle<f64, su::Logical> {
    fn from(t: Transform) -> Self {
        let (x, y, w, h) = t.to_logical();
        su::Rectangle::new(su::Point::from((x, y)), su::Size::from((w, h)))
    }
}

impl From<Transform> for su::Rectangle<i32, su::Logical> {
    fn from(t: Transform) -> Self {
        let (x, y, w, h) = t.to_logical();
        su::Rectangle::new(
            su::Point::from((x.round() as i32, y.round() as i32)),
            su::Size::from((w.round() as i32, h.round() as i32)),
        )
    }
}

impl From<Transform> for su::Rectangle<f64, su::Physical> {
    fn from(t: Transform) -> Self {
        let (x, y, w, h) = t.to_physical();
        su::Rectangle::new(su::Point::from((x, y)), su::Size::from((w, h)))
    }
}

impl From<Transform> for su::Rectangle<i32, su::Physical> {
    fn from(t: Transform) -> Self {
        let (x, y, w, h) = t.to_physical();
        su::Rectangle::new(
            su::Point::from((x.round() as i32, y.round() as i32)),
            su::Size::from((w.round() as i32, h.round() as i32)),
        )
    }
}

impl From<Transform> for su::Point<f64, su::Logical> {
    fn from(t: Transform) -> Self {
        let (x, y, _, _) = t.to_logical();
        su::Point::from((x, y))
    }
}

impl From<Transform> for su::Point<i32, su::Logical> {
    fn from(t: Transform) -> Self {
        let (x, y, _, _) = t.to_logical();
        su::Point::from((x.round() as i32, y.round() as i32))
    }
}

impl From<Transform> for su::Point<f64, su::Physical> {
    fn from(t: Transform) -> Self {
        let (x, y, _, _) = t.to_physical();
        su::Point::from((x, y))
    }
}

impl From<Transform> for su::Point<i32, su::Physical> {
    fn from(t: Transform) -> Self {
        let (x, y, _, _) = t.to_physical();
        su::Point::from((x.round() as i32, y.round() as i32))
    }
}

impl From<Transform> for su::Size<f64, su::Logical> {
    fn from(t: Transform) -> Self {
        let (_, _, w, h) = t.to_logical();
        su::Size::from((w, h))
    }
}

impl From<Transform> for su::Size<i32, su::Logical> {
    fn from(t: Transform) -> Self {
        let (_, _, w, h) = t.to_logical();
        su::Size::from((w.round() as i32, h.round() as i32))
    }
}

impl From<Transform> for su::Size<f64, su::Physical> {
    fn from(t: Transform) -> Self {
        let (_, _, w, h) = t.to_physical();
        su::Size::from((w, h))
    }
}

impl From<Transform> for su::Size<i32, su::Physical> {
    fn from(t: Transform) -> Self {
        let (_, _, w, h) = t.to_physical();
        su::Size::from((w.round() as i32, h.round() as i32))
    }
}

impl From<Transform> for (f64, f64, f64, f64) {
    /// (x, y, w, h) in screen-logical (camera applied).
    fn from(t: Transform) -> Self {
        t.to_logical()
    }
}

impl From<Transform> for (i32, i32, i32, i32) {
    fn from(t: Transform) -> Self {
        let (x, y, w, h) = t.to_logical();
        (
            x.round() as i32,
            y.round() as i32,
            w.round() as i32,
            h.round() as i32,
        )
    }
}

impl From<Transform> for (f64, f64) {
    /// (x, y) of the rect's top-left in screen-logical (camera applied).
    fn from(t: Transform) -> Self {
        let (x, y, _, _) = t.to_logical();
        (x, y)
    }
}

impl From<Transform> for (i32, i32) {
    fn from(t: Transform) -> Self {
        let (x, y, _, _) = t.to_logical();
        (x.round() as i32, y.round() as i32)
    }
}
