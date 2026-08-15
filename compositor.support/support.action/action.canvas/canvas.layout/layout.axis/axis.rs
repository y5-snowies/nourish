use compositor_support_action_canvas_layout_rect::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

#[inline]
pub fn axis_min(r: &Rect, axis: Axis) -> f64 {
    match axis {
        Axis::X => r.left(),
        Axis::Y => r.top(),
    }
}

#[inline]
pub fn axis_len(r: &Rect, axis: Axis) -> f64 {
    match axis {
        Axis::X => r.w,
        Axis::Y => r.h,
    }
}

#[inline]
pub fn axis_center(r: &Rect, axis: Axis) -> f64 {
    match axis {
        Axis::X => r.center_x(),
        Axis::Y => r.center_y(),
    }
}

#[inline]
pub fn axis_set_min(r: &mut Rect, axis: Axis, v: f64) {
    match axis {
        Axis::X => r.x = v,
        Axis::Y => r.y = v,
    }
}
