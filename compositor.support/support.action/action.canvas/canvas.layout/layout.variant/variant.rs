use compositor_support_action_canvas_layout_flags::LayoutFlags;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributeVariant {
    Default,
    Start,
    Average,
    Min,
    Max,
    Axis,
    AxisBounded,
}

pub fn distribute_variant_h(flags: LayoutFlags) -> DistributeVariant {
    use LayoutFlags as F;
    if flags.contains(F::DISTRIBUTE_TARGET_H_AXIS_BOUNDED) {
        DistributeVariant::AxisBounded
    } else if flags.contains(F::DISTRIBUTE_TARGET_H_AXIS) {
        DistributeVariant::Axis
    } else if flags.contains(F::DISTRIBUTE_TARGET_H_MAX) {
        DistributeVariant::Max
    } else if flags.contains(F::DISTRIBUTE_TARGET_H_MIN) {
        DistributeVariant::Min
    } else if flags.contains(F::DISTRIBUTE_TARGET_H_AVERAGE) {
        DistributeVariant::Average
    } else if flags.contains(F::DISTRIBUTE_TARGET_H_START) {
        DistributeVariant::Start
    } else {
        DistributeVariant::Default
    }
}

pub fn distribute_variant_v(flags: LayoutFlags) -> DistributeVariant {
    use LayoutFlags as F;
    if flags.contains(F::DISTRIBUTE_TARGET_V_AXIS_BOUNDED) {
        DistributeVariant::AxisBounded
    } else if flags.contains(F::DISTRIBUTE_TARGET_V_AXIS) {
        DistributeVariant::Axis
    } else if flags.contains(F::DISTRIBUTE_TARGET_V_MAX) {
        DistributeVariant::Max
    } else if flags.contains(F::DISTRIBUTE_TARGET_V_MIN) {
        DistributeVariant::Min
    } else if flags.contains(F::DISTRIBUTE_TARGET_V_AVERAGE) {
        DistributeVariant::Average
    } else if flags.contains(F::DISTRIBUTE_TARGET_V_START) {
        DistributeVariant::Start
    } else {
        DistributeVariant::Default
    }
}
