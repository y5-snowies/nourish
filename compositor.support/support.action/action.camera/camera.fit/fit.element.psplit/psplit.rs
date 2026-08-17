use smithay::utils::{Logical, Point, Rectangle};
use compositor_support_action_camera_find_base::find::Direction;
use compositor_support_action_camera_fit_element_flags::{CameraPlacementFlags, PlacementResult};
use compositor_support_action_camera_fit_element_pan::{PanKind, pan_for_kind};

/// The default-mode (non-dominance) per-axis split:
/// 1 strategy → applies to both axes.
/// 2 strategies → more aggressive on primary axis, more conservative on perpendicular.
///   Aggressiveness order: CENTER > CORNER > FIT.
pub fn compute_pan_with_implicit_split(
    flags: CameraPlacementFlags,
    bbox: Rectangle<f64, Logical>,
    current: PlacementResult,
    viewport: Rectangle<f64, Logical>,
    dir: Direction,
) -> Point<f64, Logical> {
    use CameraPlacementFlags as F;

    let has_center = flags.contains(F::PAN_CENTER);
    let has_fit = flags.contains(F::PAN_FIT);
    let has_corner = flags.contains(F::PAN_CORNER);

    let mut by_aggressiveness: Vec<PanKind> = Vec::new();
    if has_center { by_aggressiveness.push(PanKind::Center); }
    if has_corner { by_aggressiveness.push(PanKind::Corner); }
    if has_fit { by_aggressiveness.push(PanKind::Fit); }

    if by_aggressiveness.is_empty() {
        return current.position;
    }

    match dir {
        Direction::Up | Direction::Down | Direction::Left | Direction::Right => {
            let primary_is_x = matches!(dir, Direction::Left | Direction::Right);
            let (primary_kind, perp_kind) = match by_aggressiveness.len() {
                1 => (by_aggressiveness[0], by_aggressiveness[0]),
                _ => (by_aggressiveness[0], *by_aggressiveness.last().unwrap()),
            };
            let p_primary = pan_for_kind(primary_kind, bbox, current, viewport, dir);
            let p_perp = pan_for_kind(perp_kind, bbox, current, viewport, dir);
            if primary_is_x {
                Point::from((p_primary.x, p_perp.y))
            } else {
                Point::from((p_perp.x, p_primary.y))
            }
        }
        Direction::Diagonal(..) => {
            let (sx, sy) = dir.octant().components();
            let (primary_kind, perp_kind) = match by_aggressiveness.len() {
                1 => (by_aggressiveness[0], by_aggressiveness[0]),
                _ => (by_aggressiveness[0], *by_aggressiveness.last().unwrap()),
            };
            let p_primary = pan_for_kind(primary_kind, bbox, current, viewport, dir);
            let p_perp = pan_for_kind(perp_kind, bbox, current, viewport, dir);
            let take_x = if sx != 0 { &p_primary } else { &p_perp };
            let take_y = if sy != 0 { &p_primary } else { &p_perp };
            Point::from((take_x.x, take_y.y))
        }
    }
}
