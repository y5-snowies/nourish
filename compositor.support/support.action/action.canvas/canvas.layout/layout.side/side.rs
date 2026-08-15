use compositor_support_action_canvas_layout_rect::Rect;
use compositor_support_action_canvas_layout_minsize::MinSize;
use compositor_support_action_canvas_layout_axis::Axis;

#[derive(Debug, Clone, Copy)]
pub enum AxisAlign {
    None,
    Edge { side: Side, stretch: bool },
    DualEdgeStretch,
    Center,
    CenterAndEdgeStretch(Side),
}

#[derive(Debug, Clone, Copy)]
pub enum Side {
    Min,
    Max,
}

pub fn resolve_axis_mode(
    edge_min: bool,
    edge_max: bool,
    center: bool,
    stretch_min: bool,
    stretch_max: bool,
) -> AxisAlign {
    match (edge_min, edge_max, center) {
        (true, true, true) => AxisAlign::Center,
        (true, true, false) => AxisAlign::DualEdgeStretch,
        (true, false, _) => AxisAlign::Edge { side: Side::Min, stretch: stretch_min },
        (false, true, _) => AxisAlign::Edge { side: Side::Max, stretch: stretch_max },
        (false, false, true) => AxisAlign::Center,
        (false, false, false) => AxisAlign::None,
    }
}

pub fn apply_axis(r: &mut Rect, target: &Rect, axis: Axis, mode: AxisAlign) {
    let (pos, size, t_min, t_max) = match axis {
        Axis::X => (&mut r.x, &mut r.w, target.left(), target.right()),
        Axis::Y => (&mut r.y, &mut r.h, target.top(), target.bottom()),
    };
    match mode {
        AxisAlign::None => {}
        AxisAlign::Edge { side: Side::Min, stretch } => {
            let new_min = t_min;
            if stretch {
                let new_max = (*pos) + *size;
                *pos = new_min;
                *size = (new_max - new_min).max(0.0);
            } else {
                *pos = new_min;
            }
        }
        AxisAlign::Edge { side: Side::Max, stretch } => {
            let new_max = t_max;
            if stretch {
                *size = (new_max - *pos).max(0.0);
            } else {
                *pos = new_max - *size;
            }
        }
        AxisAlign::DualEdgeStretch => {
            *pos = t_min;
            *size = (t_max - t_min).max(0.0);
        }
        AxisAlign::Center => {
            let center = (t_min + t_max) * 0.5;
            *pos = center - *size * 0.5;
        }
        AxisAlign::CenterAndEdgeStretch(_) => unreachable!(),
    }
}

pub fn clamp_min(r: &mut Rect, min: MinSize) {
    if r.w < min.w { r.w = min.w; }
    if r.h < min.h { r.h = min.h; }
}
