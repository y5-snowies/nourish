use compositor_support_action_camera_find_flags::WindowFinderFlags;
use compositor_support_action_camera_find_passes::BasePass;

pub fn build_base_passes(flags: WindowFinderFlags) -> Vec<BasePass> {
    use WindowFinderFlags as F;
    let mut seq = Vec::with_capacity(16);
    let stretch = flags.contains(F::RAYCAST_STRETCH);

    // Group 0: Base
    if flags.contains(F::RAYCAST_BASE) {
        seq.push(BasePass::Base);
        if flags.contains(F::RAYCAST_CYCLING_BASE) {
            seq.push(BasePass::CyclingBase);
        }
    }

    // Group 1: Screen edges
    let h = flags.contains(F::RAYCAST_SCREEN_HIGH);
    let l = flags.contains(F::RAYCAST_SCREEN_LOW);
    let cyc_screen = flags.contains(F::RAYCAST_CYCLING_SCREEN);
    if h || l {
        if stretch {
            seq.push(BasePass::ScreenStretch);
            if cyc_screen { seq.push(BasePass::CyclingScreenStretch); }
        } else {
            if h { seq.push(BasePass::ScreenHigh); }
            if l { seq.push(BasePass::ScreenLow); }
            if cyc_screen { seq.push(BasePass::CyclingScreen); }
        }
    }

    // Group 2: ScreenExtra (+1 monitor each side)
    if flags.contains(F::RAYCAST_SCREEN_EXTRA) {
        if stretch {
            seq.push(BasePass::ExtraStretch);
            if flags.contains(F::RAYCAST_CYCLING_SCREEN_EXTRA) {
                seq.push(BasePass::CyclingExtraStretch);
            }
        } else {
            seq.push(BasePass::ExtraHigh);
            seq.push(BasePass::ExtraLow);
            if flags.contains(F::RAYCAST_CYCLING_SCREEN_EXTRA) {
                seq.push(BasePass::CyclingExtra);
            }
        }
    }

    // Group 3: All windows
    if flags.contains(F::RAYCAST_ALL) {
        if stretch {
            seq.push(BasePass::AllStretch);
            if flags.contains(F::RAYCAST_CYCLING_ALL) {
                seq.push(BasePass::CyclingAllStretch);
            }
        } else {
            seq.push(BasePass::AllHigh);
            seq.push(BasePass::AllLow);
            if flags.contains(F::RAYCAST_CYCLING_ALL) {
                seq.push(BasePass::CyclingAll);
            }
        }
    }
    seq
}
