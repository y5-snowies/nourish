use compositor_support_action_camera_fit_aspect_types::{Size, MIN_H, MIN_W};

/// Default mode: extend the axis that needs the least movement to hit
/// target_ratio. If neither single-axis extension fits, scale down
/// proportionally and re-extend (handled by `scale_down_to_fit`).
pub fn compute_minimal_extension(
    start: Size,
    target_ratio: f32,
    max_w: f32,
    max_h: f32,
) -> Size {
    let cand_a = Size {
        w: start.h * target_ratio,
        h: start.h,
    };
    let cand_b = Size {
        w: start.w,
        h: start.w / target_ratio,
    };

    let a_valid =
        cand_a.w >= start.w && cand_a.w <= max_w && cand_a.w >= MIN_W && cand_a.h >= MIN_H;
    let b_valid =
        cand_b.h >= start.h && cand_b.h <= max_h && cand_b.w >= MIN_W && cand_b.h >= MIN_H;

    match (a_valid, b_valid) {
        (true, true) => {
            let grow_a = cand_a.w - start.w;
            let grow_b = cand_b.h - start.h;
            if grow_a <= grow_b { cand_a } else { cand_b }
        }
        (true, false) => cand_a,
        (false, true) => cand_b,
        (false, false) => scale_down_to_fit(start, target_ratio, max_w, max_h),
    }
}

/// Fallback for minimal-extension mode when neither extension fits.
/// Shrinks proportionally to the largest target-ratio box that fits
/// inside the allowable box, never growing beyond `start`.
pub fn scale_down_to_fit(start: Size, target_ratio: f32, max_w: f32, max_h: f32) -> Size {
    let box_ratio = max_w / max_h;
    let (mut w, mut h) = if box_ratio >= target_ratio {
        (max_h * target_ratio, max_h)
    } else {
        (max_w, max_w / target_ratio)
    };

    let scale = (start.w / w).min(start.h / h).min(1.0);
    w *= scale;
    h *= scale;

    if w < MIN_W || h < MIN_H {
        return Size {
            w: w.max(MIN_W).min(max_w),
            h: h.max(MIN_H).min(max_h),
        };
    }
    Size { w, h }
}
