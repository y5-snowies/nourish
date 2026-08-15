use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_window::{WindowEntry, cmp_f64};
use compositor_support_action_camera_find_axes::DirAxes;

#[derive(Clone, Copy, Debug)]
pub struct BandState {
    pub secondary_low: f64,
    pub secondary_high: f64,
    /// Tracks which physical side of the band carries the HIGH label.
    /// True if HIGH is the geometrically-low side (top/left).
    pub high_is_top_or_left: bool,
}

/// Area of intersection between two rectangles. 0.0 if they don't overlap.
pub fn overlap_area(a: &Rectangle<f64, Logical>, b: &Rectangle<f64, Logical>) -> f64 {
    let dx = (a.loc.x + a.size.w).min(b.loc.x + b.size.w) - a.loc.x.max(b.loc.x);
    let dy = (a.loc.y + a.size.h).min(b.loc.y + b.size.h) - a.loc.y.max(b.loc.y);
    dx.max(0.0) * dy.max(0.0)
}

/// Pick the output most overlapping with the given window (by intersection area).
pub fn best_output_for<'a>(
    rect: &Rectangle<f64, Logical>,
    outputs: &'a [Rectangle<f64, Logical>],
) -> Option<&'a Rectangle<f64, Logical>> {
    outputs.iter().max_by(|a, b| {
        cmp_f64(overlap_area(rect, a), overlap_area(rect, b))
    })
}

/// Find the screen edges (low side, high side) bounding the given band.
pub fn screen_edges_for_band(
    band: &BandState,
    origin: &WindowEntry,
    outputs: &[Rectangle<f64, Logical>],
    axes: &DirAxes,
) -> (f64, f64) {
    match best_output_for(&origin.rect, outputs) {
        Some(o) => (axes.secondary_low(o), axes.secondary_high(o)),
        None => (band.secondary_low, band.secondary_high),
    }
}

/// Output dimension along the perpendicular axis.
pub fn output_perpendicular_size(
    origin: &WindowEntry,
    outputs: &[Rectangle<f64, Logical>],
    axes: &DirAxes,
) -> f64 {
    match best_output_for(&origin.rect, outputs) {
        Some(o) => axes.secondary_high(o) - axes.secondary_low(o),
        None => 0.0,
    }
}

/// Topmost (low) and bottommost (high) edges across all windows on the perpendicular axis.
pub fn all_window_edges(windows: &[WindowEntry], axes: &DirAxes) -> (f64, f64) {
    let lo = windows.iter().map(|w| axes.secondary_low(&w.rect)).fold(f64::INFINITY, f64::min);
    let hi = windows.iter().map(|w| axes.secondary_high(&w.rect)).fold(f64::NEG_INFINITY, f64::max);
    if lo.is_infinite() || hi.is_infinite() { (0.0, 0.0) } else { (lo, hi) }
}

/// Resolve which side of the band gets the HIGH (high-priority) label.
/// Returns (high_target, low_target, high_is_top_or_left).
pub fn resolve_high_low(band: &BandState, target_low: f64, target_high: f64) -> (f64, f64, bool) {
    let gap_low_side = (band.secondary_low - target_low).abs();
    let gap_high_side = (target_high - band.secondary_high).abs();
    if gap_low_side <= gap_high_side {
        (target_low, target_high, true)
    } else {
        (target_high, target_low, false)
    }
}

/// Re-anchor the ray's primary start to the trailing edge of the trailing-most window in band.
pub fn cycling_primary_start(
    band: &BandState,
    windows: &[WindowEntry],
    axes: &DirAxes,
    fallback: f64,
) -> f64 {
    let min = windows
        .iter()
        .filter(|w| {
            axes.secondary_high(&w.rect) > band.secondary_low
                && axes.secondary_low(&w.rect) < band.secondary_high
        })
        .map(|w| axes.primary_back(&w.rect))
        .fold(f64::INFINITY, f64::min);
    if min.is_infinite() { fallback } else { min }
}
