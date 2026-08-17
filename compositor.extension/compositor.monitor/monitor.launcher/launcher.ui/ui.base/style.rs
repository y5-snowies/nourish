//! Visual constants for the launcher.

use iced_core::Color;

const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

// ─── Banner ──────────────────────────────────────────────────────────

/// Deep dark teal-black. High alpha — the banner is a real surface,
/// not a translucent overlay.
pub const BANNER_BG: Color = rgba(0.05, 0.06, 0.09, 0.96);

/// Warm gold/amber rim — the launcher's signature accent.
pub const BANNER_BORDER: Color = rgba(0.93, 0.76, 0.38, 0.65);

pub const BANNER_RADIUS: f32 = 22.0;
pub const BANNER_BORDER_WIDTH: f32 = 1.8;

// ─── Typography ──────────────────────────────────────────────────────

pub const TEXT: Color = rgba(0.97, 0.97, 0.98, 1.0);
pub const TEXT_DIM: Color = rgba(0.62, 0.65, 0.71, 1.0);
pub const TEXT_FAINT: Color = rgba(0.42, 0.45, 0.51, 1.0);

pub const TEXT_SIZE_TITLE: f32 = 20.0;
pub const TEXT_SIZE_SEARCH: f32 = 16.0;
pub const TEXT_SIZE_ARROW: f32 = 16.0;
/// The per-entry carets drawn above an app that declares desktop actions.
/// Small: they annotate the icon rather than competing with it.
pub const TEXT_SIZE_ENTRY_ARROW: f32 = 9.0;

// ─── Selection accent ────────────────────────────────────────────────

/// Vivid blue-violet for the selected cell.
pub const ACCENT: Color = rgba(0.36, 0.56, 0.96, 1.0);
pub const ACCENT_RING: Color = rgba(0.62, 0.78, 1.0, 1.0);
pub const ACCENT_GLOW: Color = rgba(0.36, 0.56, 0.96, 0.55);

// ─── Carousel ────────────────────────────────────────────────────────

/// Maximum cells shown at once. If `visible` has fewer apps than this,
/// only that many cells are rendered — no ghost padding.
pub const CAROUSEL_VISIBLE: usize = 5;

/// Render size of the icon image inside a non-selected cell.
pub const ICON_PX: f32 = 56.0;

/// Slightly larger render size for the selected cell so it pops.
pub const ICON_PX_SELECTED: f32 = 64.0;

/// Fixed cell size — never changes with state. We size cells just
/// large enough to hold the selected icon comfortably.
pub const CELL_PX: f32 = 82.0;
pub const CELL_GAP: f32 = 12.0;
pub const CELL_RADIUS: f32 = 16.0;

// ─── Search chip (inline at top of banner) ───────────────────────────

pub const SEARCH_CHIP_BG: Color = rgba(0.0, 0.0, 0.0, 0.45);
pub const SEARCH_CHIP_BORDER: Color = rgba(1.0, 1.0, 1.0, 0.10);
pub const SEARCH_CHIP_RADIUS: f32 = 10.0;
pub const SEARCH_CHIP_PAD_X: u16 = 12;
pub const SEARCH_CHIP_PAD_Y: u16 = 5;

// ─── Layout ──────────────────────────────────────────────────────────

pub const BANNER_PAD_X: u16 = 22;
pub const BANNER_PAD_Y: u16 = 18;

pub const ROW_GAP: f32 = 12.0;

/// Reserved height for the inline header row that holds the search
/// chip. Always reserved so the banner doesn't shift vertically when
/// the user starts/stops typing.
pub const HEADER_BLOCK_HEIGHT: f32 = 30.0;

/// Reserved height for the title + arrow indicator block below the
/// carousel. Same reasoning — stable layout regardless of mode.
pub const FOOTER_BLOCK_HEIGHT: f32 = 24.0;

/// How far the directional arrow indicators sit from the centre of
/// the selected cell, in pixels. The arrow row in the footer needs
/// to be wide enough to fit ◀  app title  ▶ side by side.
pub const ARROW_HORIZ_GAP: f32 = 10.0;
