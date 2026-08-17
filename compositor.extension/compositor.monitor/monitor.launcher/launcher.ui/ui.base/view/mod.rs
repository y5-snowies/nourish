//! View tree for the launcher banner.
//!
//! ```text
//!   ╭─────────────────────────────────────────╮
//!   │  › chr                                  │  ← header (search chip when typing; reserved height)
//!   │                                         │
//!   │   [icon]   [ICON]   [icon]   [icon]     │  ← carousel (only real entries; no ghosts)
//!   │                                         │
//!   │     ◀ ▲ ▼ ▶  Google Chrome              │  ← footer (title + arrow hints when focused)
//!   ╰─────────────────────────────────────────╯
//! ```
//!
//! Design choices:
//! - Banner hugs its content. No outer scrim — the compositor owns
//!   the backdrop.
//! - No per-cell decoration except for the selected one; non-selected
//!   icons sit on the banner background directly.
//! - Selected cell: vivid accent fill + bright ring + soft glow. When
//!   focused, the glow intensifies and the directional arrow hints
//!   appear inline with the title.
//! - Carousel renders only as many cells as exist in `visible[]`,
//!   capped at `CAROUSEL_VISIBLE`. No ghost cells.
//! - Header and footer blocks reserve their heights so the banner
//!   doesn't resize across state changes.

pub mod view;
pub use view::*;
