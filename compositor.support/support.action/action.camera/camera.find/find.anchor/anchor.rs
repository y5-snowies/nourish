use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_direction::{Direction, Octant};
use compositor_support_action_camera_find_window::WindowEntry;

/// Anchor point for sort: the directional corner of the hits' bounding box.
pub fn sort_anchor(dir: Direction, bbox: &Rectangle<f64, Logical>) -> (f64, f64) {
    let x_min = bbox.loc.x;
    let y_min = bbox.loc.y;
    let x_max = bbox.loc.x + bbox.size.w;
    let y_max = bbox.loc.y + bbox.size.h;
    match dir {
        Direction::Up | Direction::Left => (x_min, y_min),
        Direction::Down | Direction::Right => (x_max, y_max),
        Direction::Diagonal(..) => match dir.octant() {
            Octant::UpLeft => (x_min, y_min),
            Octant::UpRight => (x_max, y_min),
            Octant::DownLeft => (x_min, y_max),
            Octant::DownRight => (x_max, y_max),
            Octant::Up | Octant::Left => (x_min, y_min),
            Octant::Down | Octant::Right => (x_max, y_max),
        },
    }
}

/// The candidate's matching-corner point for sort distance computation.
pub fn candidate_corner(dir: Direction, w: &WindowEntry) -> (f64, f64) {
    let x = w.rect.loc.x;
    let y = w.rect.loc.y;
    match dir {
        Direction::Up | Direction::Left => (x, y),
        Direction::Down | Direction::Right => (x + w.rect.size.w, y + w.rect.size.h),
        Direction::Diagonal(..) => {
            let (sx, sy) = dir.octant().components();
            let cx = if sx > 0 { x + w.rect.size.w } else if sx < 0 { x } else {
                if matches!(dir.octant(), Octant::Down) { x + w.rect.size.w } else { x }
            };
            let cy = if sy > 0 { y + w.rect.size.h } else if sy < 0 { y } else {
                if matches!(dir.octant(), Octant::Right) { y + w.rect.size.h } else { y }
            };
            (cx, cy)
        }
    }
}

/// Perpendicular axis preference key: smaller value → preferred direction.
pub fn perpendicular_key(dir: Direction, w: &WindowEntry) -> f64 {
    let x = w.rect.loc.x;
    let y = w.rect.loc.y;
    match dir {
        Direction::Up => x,
        Direction::Down => -x,
        Direction::Left => y,
        Direction::Right => -y,
        Direction::Diagonal(..) => {
            let (sx, sy) = dir.octant().components();
            let (sx, sy) = (sx as f64, sy as f64);
            -sy * x - sx * y
        }
    }
}

/// Signed offset from anchor per axis for a given candidate corner.
pub fn signed_offsets(
    dir: Direction,
    anchor_x: f64,
    anchor_y: f64,
    cx: f64,
    cy: f64,
) -> (f64, f64) {
    match dir {
        Direction::Up | Direction::Left => (cx - anchor_x, cy - anchor_y),
        Direction::Down | Direction::Right => (anchor_x - cx, anchor_y - cy),
        Direction::Diagonal(..) => {
            let oct = dir.octant();
            let (sx, sy) = oct.components();
            let (flip_x, flip_y) = match oct {
                Octant::Up | Octant::Left => (false, false),
                Octant::Down | Octant::Right => (true, true),
                _ => (sx > 0, sy > 0),
            };
            let dx = if flip_x { anchor_x - cx } else { cx - anchor_x };
            let dy = if flip_y { anchor_y - cy } else { cy - anchor_y };
            (dx, dy)
        }
    }
}
