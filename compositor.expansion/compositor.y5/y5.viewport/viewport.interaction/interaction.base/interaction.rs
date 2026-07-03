//! Viewport pointer interactions: separator-bar resize drags and floating-pane
//! move/resize drags. Pure functions over the per-output view state ([`OutputViews`])
//! — the pointer input systems (`seat.pointer.input`) call these; the drag STATE lives
//! on `OutputViews` (transient, single-active). Keeping this out of the Orchestrator
//! keeps that struct slim and makes the drag math testable without a `Loop`.
use compositor_y5_viewport_layout_base::layout;
use compositor_y5_viewport_state_base::state::{Axis, FloatingDrag, OutputViews, SeparatorDrag, Viewport};
use smithay::utils::{Physical, Point, Rectangle};

/// Physical "near an edge" margin for starting a floating drag.
const EDGE_MARGIN: f64 = 24.0;
/// Minimum floating-pane extent when resizing.
const MIN_FLOAT: f64 = 100.0;

/// If `phys` is over a separator bar of the CURRENT output, begin a separator drag and
/// return true (the caller consumes the press). `bounds` is the output's physical mode
/// rect (origin-local). Else false.
pub fn try_begin_separator(views: &mut OutputViews, bounds: Rectangle<i32, Physical>, phys: Point<f64, Physical>) -> bool {
    let computed = layout::compute(views.current_views(), bounds);
    let p = Point::<i32, Physical>::from((phys.x.round() as i32, phys.y.round() as i32));
    let Some(sep) = layout::separator_at(&computed, p) else { return false };
    let (a, b, axis, a_len, b_len) = (sep.a, sep.b, sep.axis, sep.a_len as f64, sep.b_len as f64);
    let start_along = if matches!(axis, Axis::Vertical) { phys.x } else { phys.y };
    let vp = views.current_views();
    let sum_weight = vp.root.find(a).map_or(1.0, |s| s.weight) + vp.root.find(b).map_or(1.0, |s| s.weight);
    views.separator_drag = Some(SeparatorDrag { a, b, axis, start_along, a_len, b_len, sum_weight });
    true
}

/// While a separator drag is active, redistribute the two slots' `weight`s from `phys`
/// and return true (consume the motion). Else false (no active drag).
pub fn update_separator(views: &mut OutputViews, phys: Point<f64, Physical>) -> bool {
    let Some(d) = views.separator_drag else { return false };
    let combined = d.a_len + d.b_len;
    if combined <= 1.0 {
        return true;
    }
    let along = if matches!(d.axis, Axis::Vertical) { phys.x } else { phys.y };
    let min = 40.0_f64.min(combined / 3.0);
    let new_a = (d.a_len + (along - d.start_along)).clamp(min, combined - min);
    let weight_a = d.sum_weight * (new_a / combined);
    let weight_b = d.sum_weight - weight_a;
    let vp = views.current_views_mut();
    if let Some(s) = vp.root.find_mut(d.a) {
        s.weight = weight_a;
    }
    if let Some(s) = vp.root.find_mut(d.b) {
        s.weight = weight_b;
    }
    true
}

/// End any in-progress separator drag.
pub fn end_separator(views: &mut OutputViews) {
    views.separator_drag = None;
}

/// If `phys` is near a floating pane's edge on the CURRENT output, begin a move
/// (`resize=false`) or resize (`resize=true`) drag and return true (consume). The pane
/// interior is left for window interaction. Else false.
pub fn try_begin_floating(views: &mut OutputViews, phys: Point<f64, Physical>, resize: bool) -> bool {
    let (px, py) = (phys.x, phys.y);
    let found = views.current_views().floating.iter().enumerate().rev().find_map(|(i, v)| {
        let Viewport::Floating { rect, .. } = v else { return None };
        let (x0, y0) = (rect.loc.x as f64, rect.loc.y as f64);
        let (x1, y1) = (x0 + rect.size.w as f64, y0 + rect.size.h as f64);
        if px < x0 - EDGE_MARGIN || px > x1 + EDGE_MARGIN || py < y0 - EDGE_MARGIN || py > y1 + EDGE_MARGIN {
            return None;
        }
        let (l, r) = ((px - x0).abs() <= EDGE_MARGIN, (px - x1).abs() <= EDGE_MARGIN);
        let (t, b) = ((py - y0).abs() <= EDGE_MARGIN, (py - y1).abs() <= EDGE_MARGIN);
        if l || r || t || b { Some((i, l, t, r, b, *rect)) } else { None }
    });
    let Some((index, left, top, right, bottom, start_rect)) = found else { return false };
    views.floating_drag = Some(FloatingDrag { index, resize, left, top, right, bottom, start_cursor: (px, py), start_rect });
    true
}

/// While a floating drag is active, move/resize the pane from `phys`; true = consume.
pub fn update_floating(views: &mut OutputViews, phys: Point<f64, Physical>) -> bool {
    let Some(d) = views.floating_drag else { return false };
    let (dx, dy) = (phys.x - d.start_cursor.0, phys.y - d.start_cursor.1);
    let vp = views.current_views_mut();
    let Some(Viewport::Floating { rect, .. }) = vp.floating.get_mut(d.index) else { return true };
    if d.resize {
        let (mut x0, mut y0) = (d.start_rect.loc.x as f64, d.start_rect.loc.y as f64);
        let (mut x1, mut y1) = (x0 + d.start_rect.size.w as f64, y0 + d.start_rect.size.h as f64);
        if d.left {
            x0 = (x0 + dx).min(x1 - MIN_FLOAT);
        }
        if d.right {
            x1 = (x1 + dx).max(x0 + MIN_FLOAT);
        }
        if d.top {
            y0 = (y0 + dy).min(y1 - MIN_FLOAT);
        }
        if d.bottom {
            y1 = (y1 + dy).max(y0 + MIN_FLOAT);
        }
        *rect = Rectangle::from_loc_and_size((x0.round() as i32, y0.round() as i32), ((x1 - x0).round() as i32, (y1 - y0).round() as i32));
    } else {
        *rect = Rectangle::from_loc_and_size((d.start_rect.loc.x + dx.round() as i32, d.start_rect.loc.y + dy.round() as i32), d.start_rect.size);
    }
    true
}

/// End any in-progress floating drag.
pub fn end_floating(views: &mut OutputViews) {
    views.floating_drag = None;
}
