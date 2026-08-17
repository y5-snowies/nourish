use smithay::utils::{Logical, Rectangle, Size};
use compositor_support_action_camera_fit_element_flags::{CameraPlacementFlags, PlacementResult};

#[derive(Clone, Copy, Debug)]
pub struct ZoomProposal {
    pub zoom: f64,
    #[allow(dead_code)]
    pub kind: ZoomKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomKind {
    OutToFit,
    InToFit,
    Identity,
}

pub fn collect_zoom_proposals(
    flags: CameraPlacementFlags,
    bbox: Rectangle<f64, Logical>,
    current: PlacementResult,
    screen_size: Size<f64, Logical>,
) -> Vec<ZoomProposal> {
    use CameraPlacementFlags as F;

    let consider_h =
        !flags.contains(F::ZOOM_FIT_VERTICAL) || flags.contains(F::ZOOM_FIT_HORIZONTAL);
    let consider_v =
        !flags.contains(F::ZOOM_FIT_HORIZONTAL) || flags.contains(F::ZOOM_FIT_VERTICAL);

    let zoom_for_fit = {
        let z_h = if consider_h && bbox.size.w > 0.0 {
            screen_size.w / bbox.size.w
        } else {
            f64::INFINITY
        };
        let z_v = if consider_v && bbox.size.h > 0.0 {
            screen_size.h / bbox.size.h
        } else {
            f64::INFINITY
        };
        z_h.min(z_v)
    };

    let mut props = Vec::new();

    if flags.contains(F::ZOOM_OUT_TO_FIT) {
        let z = current.zoom.min(zoom_for_fit);
        let z = z.max(0.001);
        props.push(ZoomProposal { zoom: z, kind: ZoomKind::OutToFit });
    }
    if flags.contains(F::ZOOM_IN_TO_FIT) {
        let z = zoom_for_fit.max(0.001);
        props.push(ZoomProposal { zoom: z, kind: ZoomKind::InToFit });
    }
    if props.is_empty() {
        props.push(ZoomProposal { zoom: current.zoom, kind: ZoomKind::Identity });
    }
    props
}
