//! Three-step responsive breakpoints for the placeholder surface.
//!
//! A placeholder inherits the geometry of the window it replaced and can
//! then be dragged to any size. Rather than scaling one layout continuously,
//! the view picks a discrete step from the *available* size and renders a
//! layout designed for it. Both axes are consulted — a placeholder can be wide and
//! short (or the reverse), and either dimension alone can make the roomy
//! layout unusable.
//!
//! - [`Breakpoint::Compact`] lays out as a ROW: icon on the left, two lines of
//!   text beside it, actions flush right. No title — at this size the app_id
//!   and the executable path identify the app more densely than a display
//!   name would.
//! - [`Breakpoint::Medium`] and [`Breakpoint::Full`] lay out as a centered
//!   COLUMN: icon, title, app_id, executable path, actions.
//!
//! [`MIN_W`] / [`MIN_H`] are the floor the Compact row is dimensioned to fit
//! in. They are the *layout* minimum, not a resize policy: the placeholder
//! state clamps every geometry write to them, so a placeholder can never be handed a
//! size the UI has no design for.

use iced_core::{Padding, Size};

use crate::style;

/// Narrowest placeholder the [`Breakpoint::Compact`] row is designed to fit.
///
/// Budget: 6+12 outer padding, 40 icon, 8 gap, ~106 for the two text lines,
/// 8 gap, 80 for the three 24px circular buttons (3×24 + 2×4 gaps).
pub const MIN_W: f32 = 260.0;

/// Shortest placeholder the [`Breakpoint::Compact`] row is designed to fit: 6+6
/// outer padding around a row as tall as its tallest child (the 40px icon),
/// with a few pixels of slack.
pub const MIN_H: f32 = 56.0;

/// Raise a placeholder's stored size to the layout floor.
///
/// This is the HARD rule, and it is deliberately not a resize policy: it
/// applies to every geometry write regardless of origin — the geometry a placeholder
/// inherits from the window it replaced, a record rehydrated from disk, a
/// drag — so a placeholder can never be handed a size the UI has no design
/// for. The interactive resize clamp enforces the same floor at the one place
/// the user can reach it; this catches every other path.
///
/// Sizes are the world-logical `(w, h)` the placeholder slot stores.
pub fn clamp_size(size: (i32, i32)) -> (i32, i32) {
    (size.0.max(MIN_W as i32), size.1.max(MIN_H as i32))
}

/// Which of the three layout steps the surface renders at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Breakpoint {
    /// Row: icon | app_id + exec path | actions.
    Compact,
    /// Column with a 64px icon and tightened spacing.
    Medium,
    /// Column with a 96px icon and roomy spacing.
    Full,
}

impl Breakpoint {
    /// Below this width or height the placeholder renders [`Breakpoint::Compact`].
    pub const COMPACT_W: f32 = 300.0;
    pub const COMPACT_H: f32 = 230.0;
    /// Below this width or height the placeholder renders [`Breakpoint::Medium`].
    pub const MEDIUM_W: f32 = 480.0;
    pub const MEDIUM_H: f32 = 400.0;

    /// Pick the step for the space the surface was given.
    ///
    /// An unbounded axis (`f32::INFINITY`, what an unconstrained parent hands
    /// down) compares `false` against every threshold and falls through to
    /// [`Breakpoint::Full`] — correct, since an unbounded axis is never the
    /// axis that crops us.
    pub fn of(size: Size) -> Self {
        if size.width < Self::COMPACT_W || size.height < Self::COMPACT_H {
            Self::Compact
        } else if size.width < Self::MEDIUM_W || size.height < Self::MEDIUM_H {
            Self::Medium
        } else {
            Self::Full
        }
    }

    /// Whether this step lays its content out as a row rather than a column.
    pub fn is_row(self) -> bool {
        matches!(self, Self::Compact)
    }

    /// Padding around the whole content block.
    ///
    /// Compact carries extra padding on the RIGHT: its actions are flush to
    /// that edge, and circles read as cramped against a rounded panel border
    /// with only the symmetric gutter between them.
    pub fn outer_padding(self) -> Padding {
        match self {
            Self::Compact => Padding { top: 6.0, right: 12.0, bottom: 6.0, left: 6.0 },
            Self::Medium => Padding::new(16.0),
            Self::Full => Padding::new(32.0),
        }
    }

    /// Gap between the main content blocks (icon / text / actions).
    pub fn gap(self) -> f32 {
        match self {
            Self::Compact => 8.0,
            Self::Medium => 10.0,
            Self::Full => 16.0,
        }
    }

    /// Gap between the two text lines in the Compact row.
    pub fn line_gap(self) -> f32 {
        2.0
    }

    /// Horizontal gap between the action buttons.
    pub fn button_gap(self) -> f32 {
        match self {
            Self::Compact => 4.0,
            Self::Medium => 8.0,
            Self::Full => 12.0,
        }
    }

    /// Square side of the application icon.
    pub fn icon_px(self) -> f32 {
        match self {
            Self::Compact => 40.0,
            Self::Medium => 64.0,
            Self::Full => 96.0,
        }
    }

    /// Whether the display name is shown. Compact drops it: the app_id and
    /// the executable path fit the two lines it has room for, and they
    /// identify the app more precisely.
    pub fn shows_title(self) -> bool {
        !matches!(self, Self::Compact)
    }

    /// Text size for the display name.
    pub fn title_size(self) -> f32 {
        match self {
            // Unused at Compact (see `shows_title`), kept total for callers.
            Self::Compact => style::TEXT_SIZE_HINT,
            Self::Medium => style::TEXT_SIZE_SECTION,
            Self::Full => style::TEXT_SIZE_TITLE,
        }
    }

    /// Text size for the app_id and executable-path lines.
    pub fn detail_size(self) -> f32 {
        match self {
            Self::Compact => 11.0,
            Self::Medium | Self::Full => style::TEXT_SIZE_HINT,
        }
    }

    /// Text size inside the action buttons.
    pub fn button_text_size(self) -> f32 {
        match self {
            Self::Compact | Self::Medium => style::TEXT_SIZE_HINT,
            Self::Full => style::TEXT_SIZE_BODY,
        }
    }

    /// Padding inside a WORD-labelled button.
    ///
    /// The glyph buttons of the Compact view are sized by
    /// [`Breakpoint::button_diameter`] and take no padding at all — padding
    /// would make them wider than they are tall, i.e. an ellipse. Compact still
    /// has word-labelled buttons elsewhere (the confirmation prompt), and those
    /// need this.
    pub fn button_padding(self) -> Padding {
        match self {
            Self::Compact | Self::Medium => style::PAD_SMALL,
            Self::Full => style::PAD_MEDIUM,
        }
    }

    /// Glyph size of the container badge.
    pub fn badge_px(self) -> f32 {
        match self {
            Self::Compact => 10.0,
            Self::Medium => 13.0,
            Self::Full => 16.0,
        }
    }

    /// Inset for the floating overlays (container badge, container name).
    ///
    /// Separate from [`Breakpoint::outer_padding`]: the overlays sit OUTSIDE
    /// the content box and must hug the panel's rounded border, not line up
    /// with the content.
    pub fn overlay_inset(self) -> f32 {
        match self {
            Self::Compact => 3.0,
            Self::Medium => 6.0,
            Self::Full => 10.0,
        }
    }

    /// Diameter of a Compact action button, or `None` where the buttons are
    /// word-labelled and sized by their content.
    ///
    /// Compact draws its three glyph actions as PURE CIRCLES: width and height
    /// are both this, and the corner radius is half of it. Sizing them from
    /// text + padding instead can't produce a circle — the glyphs have
    /// different advance widths, so each button would be a differently
    /// stretched ellipse.
    pub fn button_diameter(self) -> Option<f32> {
        match self {
            Self::Compact => Some(24.0),
            Self::Medium | Self::Full => None,
        }
    }

    /// Corner radius for an action button: half the diameter where the buttons
    /// are circular, the shared panel radius otherwise.
    pub fn button_radius(self) -> f32 {
        match self.button_diameter() {
            Some(d) => d / 2.0,
            None => style::RADIUS_MEDIUM,
        }
    }

    /// Label for an action button. Compact has no room for words, so the
    /// three actions collapse to glyphs.
    pub fn label(self, action: Action) -> &'static str {
        match (self, action) {
            (Self::Compact, Action::Launch) => "▶",
            (Self::Compact, Action::Edit) => "⚙",
            (Self::Compact, Action::Dismiss) => "✕",
            (_, Action::Launch) => "Launch",
            (_, Action::Edit) => "Edit",
            (_, Action::Dismiss) => "Dismiss",
        }
    }
}

/// The three actions the view-mode button row offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Launch,
    Edit,
    Dismiss,
}
