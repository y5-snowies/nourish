use smithay::utils::{Logical, Point, Rectangle};
use compositor_support_action_camera_find_base::find::Direction;
use compositor_support_action_camera_fit_element_flags::{CameraPlacementFlags, PlacementResult};
use compositor_support_action_camera_fit_element_viewport::viewport_at;

#[derive(Clone, Copy, Debug)]
pub struct PanProposal {
    pub position: Point<f64, Logical>,
    #[allow(dead_code)]
    pub kind: PanKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanKind { Center, Fit, Corner }

pub fn pan_center(bbox: Rectangle<f64, Logical>) -> Point<f64, Logical> {
    Point::from((bbox.loc.x + bbox.size.w / 2.0, bbox.loc.y + bbox.size.h / 2.0))
}

pub fn pan_fit(
    bbox: Rectangle<f64, Logical>,
    current: Point<f64, Logical>,
    viewport: Rectangle<f64, Logical>,
) -> Point<f64, Logical> {
    let mut x = current.x;
    let mut y = current.y;
    let vp_max_x = viewport.loc.x + viewport.size.w;
    let vp_max_y = viewport.loc.y + viewport.size.h;
    let bb_max_x = bbox.loc.x + bbox.size.w;
    let bb_max_y = bbox.loc.y + bbox.size.h;
    if bbox.loc.x < viewport.loc.x { x -= viewport.loc.x - bbox.loc.x; }
    else if bb_max_x > vp_max_x { x += bb_max_x - vp_max_x; }
    if bbox.loc.y < viewport.loc.y { y -= viewport.loc.y - bbox.loc.y; }
    else if bb_max_y > vp_max_y { y += bb_max_y - vp_max_y; }
    Point::from((x, y))
}

pub fn pan_corner(
    bbox: Rectangle<f64, Logical>,
    viewport: Rectangle<f64, Logical>,
    dir: Direction,
) -> Point<f64, Logical> {
    let vp_half_w = viewport.size.w / 2.0;
    let vp_half_h = viewport.size.h / 2.0;
    match dir {
        Direction::Up | Direction::Left =>
            Point::from((bbox.loc.x + vp_half_w, bbox.loc.y + vp_half_h)),
        Direction::Down | Direction::Right =>
            Point::from((bbox.loc.x + bbox.size.w - vp_half_w, bbox.loc.y + bbox.size.h - vp_half_h)),
        Direction::Diagonal(..) => {
            let (sx, sy) = dir.octant().components();
            let x = if sx > 0 { bbox.loc.x + bbox.size.w - vp_half_w }
                    else if sx < 0 { bbox.loc.x + vp_half_w }
                    else { bbox.loc.x + bbox.size.w / 2.0 };
            let y = if sy > 0 { bbox.loc.y + bbox.size.h - vp_half_h }
                    else if sy < 0 { bbox.loc.y + vp_half_h }
                    else { bbox.loc.y + bbox.size.h / 2.0 };
            Point::from((x, y))
        }
    }
}

pub fn pan_for_kind(
    kind: PanKind,
    bbox: Rectangle<f64, Logical>,
    current: PlacementResult,
    viewport: Rectangle<f64, Logical>,
    dir: Direction,
) -> Point<f64, Logical> {
    match kind {
        PanKind::Center => pan_center(bbox),
        PanKind::Fit => pan_fit(bbox, current.position, viewport),
        PanKind::Corner => pan_corner(bbox, viewport, dir),
    }
}

pub fn collect_pan_proposals(
    flags: CameraPlacementFlags,
    bbox: Rectangle<f64, Logical>,
    current: PlacementResult,
    viewport: Rectangle<f64, Logical>,
    dir: Direction,
) -> Vec<PanProposal> {
    use CameraPlacementFlags as F;
    let mut props = Vec::new();
    if flags.contains(F::PAN_CENTER) {
        props.push(PanProposal { position: pan_center(bbox), kind: PanKind::Center });
    }
    if flags.contains(F::PAN_FIT) {
        props.push(PanProposal { position: pan_fit(bbox, current.position, viewport), kind: PanKind::Fit });
    }
    if flags.contains(F::PAN_CORNER) {
        props.push(PanProposal { position: pan_corner(bbox, viewport, dir), kind: PanKind::Corner });
    }
    if props.is_empty() {
        props.push(PanProposal { position: current.position, kind: PanKind::Fit });
    }
    props
}
