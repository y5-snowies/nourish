use smithay::utils::{Logical, Rectangle};
pub use compositor_support_action_camera_find_window::WindowId;

/// Identifies which Base-phase pass produced a result, for caller-side filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BasePass {
    Base,
    CyclingBase,
    ScreenHigh,
    ScreenLow,
    CyclingScreen,
    ScreenStretch,
    CyclingScreenStretch,
    ExtraHigh,
    ExtraLow,
    CyclingExtra,
    ExtraStretch,
    CyclingExtraStretch,
    AllHigh,
    AllLow,
    CyclingAll,
    AllStretch,
    CyclingAllStretch,
}

impl BasePass {
    /// True for any cycling pass variant.
    pub fn is_cycling(self) -> bool {
        matches!(
            self,
            BasePass::CyclingBase
                | BasePass::CyclingScreen
                | BasePass::CyclingScreenStretch
                | BasePass::CyclingExtra
                | BasePass::CyclingExtraStretch
                | BasePass::CyclingAll
                | BasePass::CyclingAllStretch
        )
    }

    /// True for Stretch (combined HIGH+LOW) variants.
    pub fn is_stretch(self) -> bool {
        matches!(
            self,
            BasePass::ScreenStretch
                | BasePass::CyclingScreenStretch
                | BasePass::ExtraStretch
                | BasePass::CyclingExtraStretch
                | BasePass::AllStretch
                | BasePass::CyclingAllStretch
        )
    }
}

/// Identifies which endpoint length within a Base pass produced a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndpointPass {
    /// Ray length = origin's primary-axis length.
    WindowWidth,
    /// Ray length = origin's primary length + 1 monitor.
    OneScreen,
    /// Ray length = origin's primary length + 2 monitors.
    TwoScreens,
    /// Ray length = max forward edge over all windows (whole world reachable).
    AllTheWay,
}

/// Per-pass result, returned by `find()`.
#[derive(Clone, Debug)]
pub struct PassResult {
    pub base_pass: BasePass,
    pub endpoint: EndpointPass,
    pub ids: Vec<WindowId>,
    pub bbox: Rectangle<f64, Logical>,
}
