use smithay::utils::{Logical, Point, Rectangle, Size};
pub use compositor_support_action_camera_fit_element_flags::PlacementResult;

pub fn viewport_at(
    zoom: f64,
    pos: Point<f64, Logical>,
    screen_size: Size<f64, Logical>,
) -> Rectangle<f64, Logical> {
    let half_w = screen_size.w / (2.0 * zoom);
    let half_h = screen_size.h / (2.0 * zoom);
    Rectangle::new(
        Point::from((pos.x - half_w, pos.y - half_h)),
        Size::from((screen_size.w / zoom, screen_size.h / zoom)),
    )
}

pub fn cmp_f64(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}
