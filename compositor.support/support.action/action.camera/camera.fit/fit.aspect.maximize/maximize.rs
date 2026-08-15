use compositor_support_action_camera_fit_aspect_types::{Size, MIN_H, MIN_W};

/// Maximize: grow as large as possible at target_ratio inside the cap.
/// If a single-axis extension from start already reaches the cap, use it;
/// otherwise fill the cap directly (both axes change).
pub fn compute_maximized(start: Size, target_ratio: f32, max_w: f32, max_h: f32) -> Size {
    let box_ratio = max_w / max_h;
    let (best_w, best_h) = if box_ratio >= target_ratio {
        (max_h * target_ratio, max_h)
    } else {
        (max_w, max_w / target_ratio)
    };

    let one_axis_w = Size {
        w: start.h * target_ratio,
        h: start.h,
    };
    let one_axis_h = Size {
        w: start.w,
        h: start.w / target_ratio,
    };

    let matches_best =
        |s: Size| (s.w - best_w).abs() < 0.001 && (s.h - best_h).abs() < 0.001;

    let result = if matches_best(one_axis_w) && one_axis_w.w >= start.w && one_axis_w.w <= max_w {
        one_axis_w
    } else if matches_best(one_axis_h) && one_axis_h.h >= start.h && one_axis_h.h <= max_h {
        one_axis_h
    } else {
        Size {
            w: best_w,
            h: best_h,
        }
    };

    if result.w < MIN_W || result.h < MIN_H {
        return Size {
            w: result.w.max(MIN_W).min(max_w),
            h: result.h.max(MIN_H).min(max_h),
        };
    }
    result
}
