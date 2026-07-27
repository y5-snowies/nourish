//! Occlusion culling for the content band.
//!
//! The band already draws front-to-back (the DrawOrder authority hands out
//! topmost-first), so occlusion needs no depth pass and no second traversal:
//! each window that turns out to be opaque over a known rect deposits that rect
//! here, and every window drawn AFTER it — i.e. behind it — asks whether its own
//! rect is fully covered before doing any work.
//!
//! Kept strict on purpose. A rect is deposited only when the pixels are provably
//! opaque (an alpha-free client buffer, or the letterbox fill that covers the
//! whole slot), because a wrong deposit culls a window that should have shown
//! through. A missed deposit only costs a redundant draw.

use smithay::desktop::Window;
use smithay::utils::{Physical, Rectangle};

/// What one window contributed to the pane being drawn.
///
/// `on_pane` and `visible` are separate because their consumers want different
/// answers. Frame callbacks and presentation feedback follow `visible` — a
/// covered window contributed no pixels, so telling it its frame was presented
/// is a lie that also costs it a wakeup. Fractional scale follows `on_pane`: a
/// covered window is revealed the instant the occluder moves or closes, with no
/// pan to hide a re-publish behind, so it must keep its real scale.
#[derive(Default)]
pub struct Drawn {
    /// The window's slot overlaps the pane — on screen geometrically, whether or
    /// not anything is stacked over it.
    pub on_pane: bool,
    /// The window actually contributed pixels: on the pane AND not fully covered.
    pub visible: bool,
    /// Rect this window is opaque over, if it is opaque at all.
    pub occluder: Option<Rectangle<i32, Physical>>,
}

/// The opaque rects deposited by nearer windows, in output-physical space.
#[derive(Default)]
pub struct Occluders {
    rect: Vec<Rectangle<i32, Physical>>,
}

impl Occluders {
    /// One per pane: a rect only occludes within the viewport it was drawn into.
    pub fn new() -> Self {
        Self::default()
    }

    /// Is `rect` fully covered by the union of what has been deposited?
    ///
    /// Exact rather than "contained in any single occluder": two tiled windows
    /// side by side hide a third behind them, and that is the ordinary case on a
    /// split viewport. `subtract_rects` splits the remainder, so an empty result
    /// means every pixel is accounted for.
    pub fn hidden(&self, rect: Rectangle<i32, Physical>) -> bool {
        !self.rect.is_empty() && rect.subtract_rects(self.rect.iter().copied()).is_empty()
    }

    pub fn push(&mut self, rect: Rectangle<i32, Physical>) {
        if rect.size.w > 0 && rect.size.h > 0 {
            self.rect.push(rect);
        }
    }
}

/// The two sets a pane produces, accumulated across its windows.
#[derive(Default)]
pub struct Visible {
    /// Drew pixels this frame → frame callbacks, presentation feedback, and the
    /// tearing/pacing scene.
    pub drawn: Vec<Window>,
    /// Passed the frustum, covered or not → the fractional-scale set.
    pub on_pane: Vec<Window>,
}

impl Visible {
    pub fn note(&mut self, window: &Window, drawn: &Drawn) {
        if drawn.visible {
            self.drawn.push(window.clone());
        }
        if drawn.on_pane {
            self.on_pane.push(window.clone());
        }
    }
}
