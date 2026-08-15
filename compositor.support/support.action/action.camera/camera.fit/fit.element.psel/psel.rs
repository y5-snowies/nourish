use smithay::utils::{Logical, Point, Rectangle, Size};
use compositor_support_action_camera_fit_element_flags::{CameraPlacementFlags, PlacementResult};
use compositor_support_action_camera_fit_element_viewport::{viewport_at, cmp_f64};
use compositor_support_action_camera_fit_element_pan::{PanProposal, pan_fit};

pub fn select_best_pan(
    flags: CameraPlacementFlags,
    proposals: &[PanProposal],
    current: PlacementResult,
    bbox: Rectangle<f64, Logical>,
    target_zoom: f64,
    screen_size: Size<f64, Logical>,
) -> Point<f64, Logical> {
    use CameraPlacementFlags as F;

    let viewport_diag = (screen_size.w * screen_size.w + screen_size.h * screen_size.h).sqrt();

    proposals
        .iter()
        .map(|p| {
            let mut score = 0.0;
            let mut weight = 0.0;
            let vp = viewport_at(target_zoom, p.position, screen_size);

            if flags.contains(F::PAN_GOAL_MIN_MOVEMENT) {
                let dx = p.position.x - current.position.x;
                let dy = p.position.y - current.position.y;
                let dist = (dx * dx + dy * dy).sqrt();
                let world_diag = viewport_diag / target_zoom;
                score += 1.0 - (dist / world_diag).min(1.0);
                weight += 1.0;
            }
            if flags.contains(F::PAN_GOAL_MAX_VISIBILITY) {
                let visible_w =
                    (vp.loc.x + vp.size.w).min(bbox.loc.x + bbox.size.w) - vp.loc.x.max(bbox.loc.x);
                let visible_h =
                    (vp.loc.y + vp.size.h).min(bbox.loc.y + bbox.size.h) - vp.loc.y.max(bbox.loc.y);
                let visible = visible_w.max(0.0) * visible_h.max(0.0);
                let total = bbox.size.w * bbox.size.h;
                score += (if total > 0.0 { visible / total } else { 0.0 }).clamp(0.0, 1.0);
                weight += 1.0;
            }
            if flags.contains(F::PAN_GOAL_NO_CUTOFF) {
                let fully_visible = bbox.loc.x >= vp.loc.x && bbox.loc.y >= vp.loc.y
                    && bbox.loc.x + bbox.size.w <= vp.loc.x + vp.size.w
                    && bbox.loc.y + bbox.size.h <= vp.loc.y + vp.size.h;
                score += if fully_visible { 1.0 } else { 0.0 };
                weight += 1.0;
            }
            if flags.contains(F::PAN_GOAL_NO_OVERSHOOT) {
                let fit_target = pan_fit(bbox, current.position, vp);
                let need_dx = fit_target.x - current.position.x;
                let need_dy = fit_target.y - current.position.y;
                let need = (need_dx * need_dx + need_dy * need_dy).sqrt();
                let dx = p.position.x - current.position.x;
                let dy = p.position.y - current.position.y;
                let actual = (dx * dx + dy * dy).sqrt();
                let s = if need >= actual { 1.0 }
                    else { 1.0 - ((actual - need) / actual.max(1.0)).min(1.0) };
                score += s;
                weight += 1.0;
            }

            (p.position, if weight > 0.0 { score / weight } else { 0.0 })
        })
        .max_by(|a, b| cmp_f64(a.1, b.1))
        .map(|(pos, _)| pos)
        .unwrap_or(current.position)
}
