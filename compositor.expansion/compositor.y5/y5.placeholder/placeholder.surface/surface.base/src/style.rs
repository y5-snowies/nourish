//! Visual constants used across the placeholder UI.

use iced_core::{Color, Padding};

// Background tones
pub const BG: Color = Color { r: 0.04, g: 0.05, b: 0.07, a: 1.0 };
/// Panel background when the placeholder's app runs inside a container.
/// Violet-shifted rather than brighter, so the placeholder reads as "different
/// place" at a glance without competing with the selection ring.
pub const BG_CONTAINER: Color = Color { r: 0.08, g: 0.05, b: 0.13, a: 1.0 };
/// Panel background when the placeholder's client declares an
/// `xdg-session-management` identity. Teal-shifted — a different axis from the
/// container violet, so the two are told apart by hue rather than brightness and
/// neither reads as "more important".
pub const BG_SESSION: Color = Color { r: 0.03, g: 0.08, b: 0.09, a: 1.0 };
/// Both at once. Carries the container violet with the session teal lifted into
/// it, so a placeholder that is both is recognisably neither of the single states — the
/// point is that you can tell all four apart, not that they blend.
pub const BG_CONTAINER_SESSION: Color = Color { r: 0.06, g: 0.08, b: 0.14, a: 1.0 };
/// Warm the chosen panel background for a placeholder whose window was an X11
/// client.
///
/// A MODIFIER rather than a fifth constant, on purpose. The four tones above are a
/// closed set chosen so all four are told apart, and X11-ness is an independent
/// axis — it can coexist with any of them, so hand-picking eight tones would make
/// each one less distinguishable to encode a fact that is really just "and also".
/// Lifting red and dropping blue-green a little reads as a warm cast over whatever
/// the base was saying, which is what "and also" should look like.
///
/// Deliberately gentle: this marks a protocol difference, not a problem, and it
/// must not compete with the selection ring or the launching state.
pub fn with_x11_tint(base: Color) -> Color {
    Color {
        r: (base.r + 0.055).min(1.0),
        g: (base.g - 0.008).max(0.0),
        b: (base.b - 0.012).max(0.0),
        a: base.a,
    }
}

pub const PANEL_BG: Color = Color { r: 0.08, g: 0.09, b: 0.12, a: 1.0 };
pub const PANEL_BG_SOFT: Color = Color { r: 0.10, g: 0.12, b: 0.16, a: 0.85 };

// Text
pub const TEXT: Color = Color { r: 0.92, g: 0.94, b: 0.97, a: 1.0 };
pub const TEXT_DIM: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.55 };
pub const TEXT_HINT: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.35 };

// Lines / accents
pub const BORDER: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 };
pub const BORDER_BRIGHT: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.22 };
pub const ACCENT: Color = Color { r: 0.40, g: 0.65, b: 0.95, a: 1.0 };
pub const GLOW: Color = Color { r: 0.40, g: 0.65, b: 0.95, a: 0.30 };

// Container markers (badge + name floater)
pub const CONTAINER_ACCENT: Color = Color { r: 0.74, g: 0.58, b: 0.97, a: 1.0 };
/// Used for the "name unavailable" state — same hue, clearly weaker, so the
/// floater reads as present-but-unknown rather than as a name.
pub const CONTAINER_DIM: Color = Color { r: 0.74, g: 0.58, b: 0.97, a: 0.50 };

// Session markers (badge)
pub const SESSION_ACCENT: Color = Color { r: 0.42, g: 0.85, b: 0.78, a: 1.0 };

// Icon backdrop (icon container in view mode)
pub const ICON_BG: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 };
pub const ICON_HIGHLIGHT: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.18 };

// Padding
pub const PAD_SMALL: Padding = Padding { top: 6.0, right: 10.0, bottom: 6.0, left: 10.0 };
pub const PAD_MEDIUM: Padding = Padding { top: 10.0, right: 14.0, bottom: 10.0, left: 14.0 };
pub const PAD_LARGE: Padding = Padding { top: 24.0, right: 24.0, bottom: 24.0, left: 24.0 };
pub const PAD_XLARGE: Padding = Padding { top: 36.0, right: 36.0, bottom: 36.0, left: 36.0 };

// Border radii
pub const RADIUS_SMALL: f32 = 6.0;
pub const RADIUS_MEDIUM: f32 = 10.0;
pub const RADIUS_LARGE: f32 = 16.0;

// Text sizes
pub const TEXT_SIZE_TITLE: f32 = 22.0;
pub const TEXT_SIZE_SECTION: f32 = 16.0;
pub const TEXT_SIZE_BODY: f32 = 14.0;
pub const TEXT_SIZE_HINT: f32 = 12.0;
