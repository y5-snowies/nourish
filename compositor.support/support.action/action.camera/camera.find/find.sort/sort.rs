use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_direction::Direction;
use compositor_support_action_camera_find_window::{WindowEntry, WindowId, cmp_f64};
use compositor_support_action_camera_find_flags::WindowFinderFlags;
use compositor_support_action_camera_find_anchor::{candidate_corner, perpendicular_key, signed_offsets, sort_anchor};

pub fn sort_results(
    mut hits: Vec<WindowEntry>,
    flags: WindowFinderFlags,
    dir: Direction,
    origin: &WindowEntry,
    bbox: &Rectangle<f64, Logical>,
) -> Vec<WindowId> {
    use WindowFinderFlags as F;
    if flags.contains(F::SORT_NEAREST) {
        let ox = origin.rect.loc.x + origin.rect.size.w / 2.0;
        let oy = origin.rect.loc.y + origin.rect.size.h / 2.0;
        hits.sort_by(|a, b| {
            let acx = a.rect.loc.x + a.rect.size.w / 2.0;
            let acy = a.rect.loc.y + a.rect.size.h / 2.0;
            let bcx = b.rect.loc.x + b.rect.size.w / 2.0;
            let bcy = b.rect.loc.y + b.rect.size.h / 2.0;
            let adx = acx - ox; let ady = acy - oy;
            let bdx = bcx - ox; let bdy = bcy - oy;
            cmp_f64(adx * adx + ady * ady, bdx * bdx + bdy * bdy)
        });
        return hits.into_iter().map(|w| w.id).collect();
    }
    let use_x = flags.contains(F::SORT_AXIS_ORIGIN_X);
    let use_y = flags.contains(F::SORT_AXIS_ORIGIN_Y);
    if !use_x && !use_y {
        return hits.into_iter().map(|w| w.id).collect();
    }
    let (anchor_x, anchor_y) = sort_anchor(dir, bbox);
    hits.sort_by(|a, b| {
        let (ax, ay) = candidate_corner(dir, a);
        let (bx, by) = candidate_corner(dir, b);
        let (a_dx, a_dy) = signed_offsets(dir, anchor_x, anchor_y, ax, ay);
        let (b_dx, b_dy) = signed_offsets(dir, anchor_x, anchor_y, bx, by);
        let prim_a = match (use_x, use_y) {
            (true, true) => a_dx + a_dy,
            (true, false) => a_dx,
            (false, true) => a_dy,
            (false, false) => unreachable!(),
        };
        let prim_b = match (use_x, use_y) {
            (true, true) => b_dx + b_dy,
            (true, false) => b_dx,
            (false, true) => b_dy,
            (false, false) => unreachable!(),
        };
        let cmp = cmp_f64(prim_a, prim_b);
        if cmp != std::cmp::Ordering::Equal { return cmp; }
        if use_x && use_y {
            let euc_a = a_dx * a_dx + a_dy * a_dy;
            let euc_b = b_dx * b_dx + b_dy * b_dy;
            let cmp = cmp_f64(euc_a, euc_b);
            if cmp != std::cmp::Ordering::Equal { return cmp; }
        }
        cmp_f64(perpendicular_key(dir, a), perpendicular_key(dir, b))
    });
    hits.into_iter().map(|w| w.id).collect()
}
