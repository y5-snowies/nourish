use smithay::utils::{Logical, Point, Rectangle, Size};
use compositor_support_action_camera_find_window::{SYNTHETIC_ORIGIN_ID, WindowEntry};
use compositor_support_action_camera_find_band::overlap_area;

/// Build a synthetic 100×100 origin centered on the given output.
pub fn synthesize_origin(output: &Rectangle<f64, Logical>) -> WindowEntry {
    const SIZE: f64 = 100.0;
    let cx = output.loc.x + output.size.w / 2.0 - SIZE / 2.0;
    let cy = output.loc.y + output.size.h / 2.0 - SIZE / 2.0;
    WindowEntry {
        id: SYNTHETIC_ORIGIN_ID,
        rect: Rectangle::new(Point::from((cx, cy)), Size::from((SIZE, SIZE))),
    }
}

/// True when the window's rect overlaps any output rect.
pub fn is_window_visible(rect: &Rectangle<f64, Logical>, outputs: &[Rectangle<f64, Logical>]) -> bool {
    outputs.iter().any(|o| overlap_area(rect, o) > 0.0)
}

/// Visible area of a window divided by total area.
pub fn visible_area_fraction(
    rect: &Rectangle<f64, Logical>,
    outputs: &[Rectangle<f64, Logical>],
) -> f64 {
    let total = rect.size.w * rect.size.h;
    if total <= 0.0 { return 0.0; }
    let visible: f64 = outputs.iter().map(|o| overlap_area(rect, o)).sum();
    (visible / total).clamp(0.0, 1.0)
}

/// Squared distance from a window's center to the geometric center of the union of all outputs.
pub fn distance_sq_to_viewport_center(
    rect: &Rectangle<f64, Logical>,
    outputs: &[Rectangle<f64, Logical>],
) -> f64 {
    if outputs.is_empty() { return f64::INFINITY; }
    let x_min = outputs.iter().map(|o| o.loc.x).fold(f64::INFINITY, f64::min);
    let y_min = outputs.iter().map(|o| o.loc.y).fold(f64::INFINITY, f64::min);
    let x_max = outputs.iter().map(|o| o.loc.x + o.size.w).fold(f64::NEG_INFINITY, f64::max);
    let y_max = outputs.iter().map(|o| o.loc.y + o.size.h).fold(f64::NEG_INFINITY, f64::max);
    let cx = (x_min + x_max) * 0.5;
    let cy = (y_min + y_max) * 0.5;
    let wcx = rect.loc.x + rect.size.w * 0.5;
    let wcy = rect.loc.y + rect.size.h * 0.5;
    let dx = wcx - cx;
    let dy = wcy - cy;
    dx * dx + dy * dy
}
