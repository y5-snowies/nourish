//! Who a policy section applies to, and who may drive the redraw loop.
//!
//! Both resolve GLOBALLY per frame against the visible set (as `space` tracks it
//! — no culling checks), never per-surface. A page flip is per-CRTC, so "this
//! window tears and that one doesn't" is not expressible; the only meaningful
//! question is which policy governs the output this frame.
//!
//! UI strings live in `y5.graphic/graphic.tearing/tearing.text`.

use serde::{Deserialize, Serialize};

/// What the compositor sees this frame — the inputs every rule resolves against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene {
    /// A tagged ("target") window is in the drawn set.
    pub target_visible: bool,
    /// A tagged window holds keyboard focus.
    pub target_focused: bool,
    /// Anything at all holds keyboard focus.
    pub any_focused: bool,
    /// The scene drew at least one window.
    pub any_visible: bool,
}

/// When a policy section is in force.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Selector {
    #[default]
    Never,
    Always,
    Target,
    TargetFocused,
}

impl Selector {
    pub const ALL: [Selector; 4] =
        [Self::Never, Self::Always, Self::Target, Self::TargetFocused];

    pub fn matches(self, s: Scene) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::Target => s.target_visible,
            Self::TargetFocused => s.target_focused,
        }
    }
}

/// Which surfaces may schedule a redraw. One global gate — there is one loop, so
/// there is one answer.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Exclusivity {
    #[default]
    None,
    Exclusive,
    ExclusiveFocused,
    Focused,
    /// Everything the scene actually drew. Wider than `Exclusive`, but still
    /// silences offscreen clients and the compositor's own animation sources.
    Visible,
}

impl Exclusivity {
    pub const ALL: [Exclusivity; 5] =
        [Self::None, Self::Exclusive, Self::ExclusiveFocused, Self::Focused, Self::Visible];

    /// Nothing focused disengages the `*Focused` variants — that is what makes
    /// them self-releasing: normal scheduling returns on focus loss, and the
    /// floor watchdog (which exists only to rescue a starved loop) stops with it.
    pub fn engaged(self, s: Scene) -> bool {
        match self {
            Self::None => false,
            Self::Exclusive => s.target_visible,
            Self::ExclusiveFocused => s.target_focused,
            Self::Focused => s.any_focused,
            // Needs something drawn, or the gate would admit nothing at all and
            // starve the loop to the floor watchdog — the same self-releasing
            // property the focus variants have.
            Self::Visible => s.any_visible,
        }
    }
}
