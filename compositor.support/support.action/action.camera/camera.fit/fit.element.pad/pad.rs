use smithay::utils::{Logical, Point, Rectangle, Size};
use compositor_support_action_camera_find_base::find::Direction;
use compositor_support_action_camera_fit_element_viewport::viewport_at;

pub fn apply_default_padding(
    target_pos: Point<f64, Logical>,
    target_zoom: f64,
    bbox: Rectangle<f64, Logical>,
    screen_size: Size<f64, Logical>,
    dir: Direction,
) -> Point<f64, Logical> {
    let pad = default_padding(screen_size) / target_zoom;
    let vp = viewport_at(target_zoom, target_pos, screen_size);

    if bbox.size.w >= vp.size.w || bbox.size.h >= vp.size.h {
        return target_pos;
    }

    let mut x = target_pos.x;
    let mut y = target_pos.y;

    match dir {
        Direction::Right => {
            let dist = (vp.loc.x + vp.size.w) - (bbox.loc.x + bbox.size.w);
            if dist < pad { x += pad - dist; }
        }
        Direction::Left => {
            let dist = bbox.loc.x - vp.loc.x;
            if dist < pad { x -= pad - dist; }
        }
        Direction::Down => {
            let dist = (vp.loc.y + vp.size.h) - (bbox.loc.y + bbox.size.h);
            if dist < pad { y += pad - dist; }
        }
        Direction::Up => {
            let dist = bbox.loc.y - vp.loc.y;
            if dist < pad { y -= pad - dist; }
        }
        Direction::Diagonal(..) => {
            let (sx, sy) = dir.octant().components();
            if sx > 0 {
                let dist = (vp.loc.x + vp.size.w) - (bbox.loc.x + bbox.size.w);
                if dist < pad { x += pad - dist; }
            } else if sx < 0 {
                let dist = bbox.loc.x - vp.loc.x;
                if dist < pad { x -= pad - dist; }
            }
            if sy > 0 {
                let dist = (vp.loc.y + vp.size.h) - (bbox.loc.y + bbox.size.h);
                if dist < pad { y += pad - dist; }
            } else if sy < 0 {
                let dist = bbox.loc.y - vp.loc.y;
                if dist < pad { y -= pad - dist; }
            }
        }
    }
    Point::from((x, y))
}

pub fn default_padding(screen_size: Size<f64, Logical>) -> f64 {
    let min_dim = screen_size.w.min(screen_size.h);
    (min_dim * 0.05).clamp(100.0, 500.0)
}
