//! Application launcher overlay UI for the y5 compositor.
//!
//! A small, host-embeddable iced UI: a black banner of application
//! icons in the centre of the screen. Typing filters with a
//! subsequence-based fuzzy search; arrow keys move the selection;
//! Enter focuses an icon; the next arrow key emits
//! [`LauncherMessage::Launch`] tagged with a spawn direction. Escape
//! unwinds one level at a time and ultimately emits
//! [`LauncherMessage::Exit`].
//!
//! ## Embedding
//!
//! The crate exposes [`Launcher`], which implements
//! [`compositor_support_iced_core_engine_base::IcedUi`]. The compositor wires it into the
//! dmabuf runtime exactly like every other y5 UI:
//!
//! ```ignore
//! let mut launcher = Launcher::new(app_list);
//!
//! // Each frame of the compositor's loop:
//! //  1. Build/update the iced surface as usual.
//! //  2. Dispatch incoming xkb events as `LauncherMessage::Event`.
//! //  3. After dispatch, drain compositor-facing emissions:
//! for msg in launcher.take_emissions() {
//!     match msg {
//!         LauncherMessage::Launch { bin, args, direction, .. } => {
//!             spawn(bin, args, direction);
//!         }
//!         LauncherMessage::Exit => {
//!             tear_down_overlay();
//!         }
//!         _ => unreachable!("take_emissions only yields Launch / Exit"),
//!     }
//! }
//! ```
//!
//! ## Scoring
//!
//! - Default (empty query): sorted by Firefox-style frecency
//!   (log-compressed usage count combined with a 14-day-halflife
//!   recency decay).
//! - Search: subsequence match score (contiguous-run bonus + early
//!   anchor + length bonus) combined with a milder frecency component,
//!   so relevance wins but ties go to apps the user actually launches.
//!
//! See [`search`] for the full algorithm and tests.

mod message;
mod model;
mod search;
mod style;
mod ui;
mod view;

pub use message::LauncherMessage;
pub use model::{AppEntry, Application, Direction};
pub use ui::Launcher;

// Re-export the style module so the compositor can match its banner
// colours / paddings if it wants to draw a chrome around the overlay.
// pub use style as theme;
