use smithay::utils::{Logical, Rectangle, Size};
use compositor_support_action_camera_fit_element_flags::{CameraPlacementFlags, PlacementResult};
use compositor_support_action_camera_fit_element_viewport::viewport_at;
use compositor_support_action_camera_fit_element_zoom::ZoomProposal;
pub use compositor_support_action_camera_fit_element_viewport::cmp_f64;

pub fn select_best_zoom(
    flags: CameraPlacementFlags,
    proposals: &[ZoomProposal],
    current: PlacementResult,
    bbox: Rectangle<f64, Logical>,
    screen_size: Size<f64, Logical>,
) -> f64 {
    use CameraPlacementFlags as F;

    proposals
        .iter()
        .map(|p| {
            let mut score = 0.0;
            let mut weight = 0.0;
            if flags.contains(F::ZOOM_GOAL_MIN_CHANGE) {
                let ratio = (p.zoom / current.zoom).ln().abs();
                score += 1.0 - ratio.min(1.0);
                weight += 1.0;
            }
            if flags.contains(F::ZOOM_GOAL_FILL_VIEWPORT) {
                let vp = viewport_at(p.zoom, current.position, screen_size);
                let fill = (bbox.size.w * bbox.size.h) / (vp.size.w * vp.size.h);
                let ideal = 0.7;
                score += 1.0 - (fill - ideal).abs().min(1.0);
                weight += 1.0;
            }
            if flags.contains(F::ZOOM_GOAL_NO_CROP) {
                let vp = viewport_at(p.zoom, current.position, screen_size);
                let fits = vp.size.w >= bbox.size.w && vp.size.h >= bbox.size.h;
                score += if fits { 1.0 } else { 0.0 };
                weight += 1.0;
            }
            (p.zoom, if weight > 0.0 { score / weight } else { 0.0 })
        })
        .max_by(|a, b| cmp_f64(a.1, b.1))
        .map(|(z, _)| z)
        .unwrap_or(current.zoom)
}
