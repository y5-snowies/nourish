use smithay::utils::{Logical, Point, Rectangle, Size};
use compositor_support_action_camera_find_window::{WindowEntry, WindowId};
use compositor_support_action_camera_find_axes::DirAxes;
use compositor_support_action_camera_find_band::BandState;

/// Returns windows whose primary extent overlaps [primary_start, primary_end]
/// AND whose perpendicular extent overlaps the band. Excludes the origin.
pub fn cast_ray(
    windows: &[WindowEntry],
    band: &BandState,
    primary_start: f64,
    primary_end: f64,
    origin_id: WindowId,
    axes: &DirAxes,
) -> Vec<WindowEntry> {
    windows
        .iter()
        .filter(|w| w.id != origin_id)
        .filter(|w| {
            let pb = axes.primary_back(&w.rect);
            let pf = axes.primary_forward(&w.rect);
            pf > primary_start && pb < primary_end
        })
        .filter(|w| {
            axes.secondary_high(&w.rect) > band.secondary_low
                && axes.secondary_low(&w.rect) < band.secondary_high
        })
        .cloned()
        .collect()
}

/// Bounding box of the hits' rectangles. None if hits is empty.
pub fn hits_bounding_box(hits: &[WindowEntry]) -> Option<Rectangle<f64, Logical>> {
    if hits.is_empty() { return None; }
    let x_min = hits.iter().map(|w| w.rect.loc.x).fold(f64::INFINITY, f64::min);
    let y_min = hits.iter().map(|w| w.rect.loc.y).fold(f64::INFINITY, f64::min);
    let x_max = hits.iter().map(|w| w.rect.loc.x + w.rect.size.w).fold(f64::NEG_INFINITY, f64::max);
    let y_max = hits.iter().map(|w| w.rect.loc.y + w.rect.size.h).fold(f64::NEG_INFINITY, f64::max);
    Some(Rectangle::new(
        Point::from((x_min, y_min)),
        Size::from((x_max - x_min, y_max - y_min)),
    ))
}
