//! # y5_placeholder_surface
//!
//! Iced UI for a closed-window placeholder.
//!
//! Two modes:
//! - **View**: icon + display name + app_id + Launch / Edit buttons.
//! - **Settings**: scrollable form showing every attribute that applies
//!   to the active handler, with per-attribute enable toggle, current
//!   value editor, "best value" hint underneath, and a handler picker
//!   to switch the active handler.
//!
//! The whole surface is wrapped in a Scrollable so cropping by the
//! compositor never blocks any control.
//!
//! ## Boundary with the compositor
//!
//! - The compositor passes in a [`LaunchPlan`] and an
//!   `Arc<HandlerRegistry>` at construction.
//! - The UI emits [`PlaceholderMessage::LaunchClicked`] and
//!   [`PlaceholderMessage::SaveClicked { updated_plan }`] for the
//!   compositor to act on. The compositor decides what "save" means
//!   (e.g., update its placeholder struct, persist to disk, etc.).
//! - The compositor pushes [`PlaceholderMessage::UpdatePlan`] to refresh
//!   the canonical plan after it has been updated externally.
//!
//! This crate does NOT know about a Placeholder struct.

pub mod breakpoint;
pub mod message;
pub mod mode;
pub mod style;
pub mod ui;
pub mod view;

pub use message::PlaceholderMessage;
pub use ui::PlaceholderUi;
