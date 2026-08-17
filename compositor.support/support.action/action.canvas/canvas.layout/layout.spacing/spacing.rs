use compositor_support_action_canvas_layout_variant::DistributeVariant;

/// Compute target spacing for distribute-without-primary.
/// Returns `(target_spacing, allow_overlap)`.
pub fn pick_no_primary_spacing(
    variant: DistributeVariant,
    gaps: &[f64],
    total_size: f64,
    bb_min: f64,
    bb_max: f64,
    n: usize,
) -> (f64, bool) {
    match variant {
        DistributeVariant::Default => {
            let avg = if gaps.is_empty() { 0.0 } else {
                gaps.iter().sum::<f64>() / gaps.len() as f64
            };
            (avg.max(0.0), false)
        }
        DistributeVariant::Start => (0.0, false),
        DistributeVariant::Average => {
            let avg = if gaps.is_empty() { 0.0 } else {
                gaps.iter().sum::<f64>() / gaps.len() as f64
            };
            (avg, true)
        }
        DistributeVariant::Min => {
            let nn: Vec<f64> = gaps.iter().copied().filter(|g| *g >= 0.0).collect();
            let sp = if nn.is_empty() { 0.0 } else {
                nn.iter().cloned().fold(f64::INFINITY, f64::min)
            };
            (sp, true)
        }
        DistributeVariant::Max => {
            let sp = gaps.iter().cloned().fold(f64::NEG_INFINITY, f64::max).max(0.0);
            (sp, true)
        }
        DistributeVariant::Axis => {
            let extent = bb_max - bb_min;
            let sp = if n >= 2 { (extent - total_size) / (n as f64 - 1.0) } else { 0.0 };
            (sp.max(0.0), false)
        }
        DistributeVariant::AxisBounded => {
            let extent = bb_max - bb_min;
            let sp = if n >= 2 { (extent - total_size) / (n as f64 - 1.0) } else { 0.0 };
            (sp, true)
        }
    }
}

/// Compute target spacing for distribute-with-primary.
/// Returns `(target_spacing, allow_overlap)`.
pub fn pick_primary_spacing(variant: DistributeVariant, gaps: &[f64]) -> (f64, bool) {
    match variant {
        DistributeVariant::Default => {
            let avg = if gaps.is_empty() { 0.0 } else {
                gaps.iter().sum::<f64>() / gaps.len() as f64
            };
            (avg.max(0.0), false)
        }
        DistributeVariant::Start => (0.0, false),
        DistributeVariant::Average => {
            let avg = if gaps.is_empty() { 0.0 } else {
                gaps.iter().sum::<f64>() / gaps.len() as f64
            };
            (avg, true)
        }
        DistributeVariant::Min => {
            let nn: Vec<f64> = gaps.iter().copied().filter(|g| *g >= 0.0).collect();
            if nn.is_empty() { (0.0, true) } else {
                (nn.iter().cloned().fold(f64::INFINITY, f64::min), true)
            }
        }
        DistributeVariant::Max => (
            gaps.iter().cloned().fold(f64::NEG_INFINITY, f64::max).max(0.0),
            true,
        ),
        DistributeVariant::Axis | DistributeVariant::AxisBounded => (0.0, false),
    }
}
