//! Messages flowing through the launcher's iced `update` loop.
//!
//! Redux-shaped, with three pure functions:
//!
//! - **`event_process`** — pure `(state, iced_event) → Vec<message>`,
//!   the event decoder. Runs in the runtime's phase 0 for each event
//!   the UI subscribed to.
//! - **`update`** — pure reducer; mutates state from typed actions,
//!   never re-emits.
//! - **`process`** — pure `(post-state, message) → Vec<message>`,
//!   returns follow-up actions for the same tick's queue.
//!
//! Raw iced events do **not** appear as message variants. They have a
//! separate dispatch path (`subscribe` + `event_process`) so the
//! launcher can observe them without `iced_core::Event` leaking into
//! the message type.
//!
//! Categories:
//!
//! - **Compositor-handled.** The runtime's `message_handler` reacts;
//!   the reducer drops them.
//!     - [`LauncherMessage::Launch`]
//!     - [`LauncherMessage::Exit`]
//!
//! - **State actions.** Pure reducer verbs.
//!     - [`LauncherMessage::MoveCursor`]
//!     - [`LauncherMessage::FocusSelection`]
//!     - [`LauncherMessage::UnfocusSelection`]
//!     - [`LauncherMessage::ClearQuery`]
//!     - [`LauncherMessage::Backspace`]
//!     - [`LauncherMessage::AppendText`]
//!
//! - **Compositor-pushed.** Externally dispatched.
//!     - [`LauncherMessage::Tick`]
//!     - [`LauncherMessage::SetApps`]

use std::path::PathBuf;
use std::sync::Arc;

use crate::model::Direction;

/// Top-level message for the launcher UI.
#[derive(Debug, Clone)]
pub enum LauncherMessage {
    // ─── Compositor-handled ─────────────────────────────────────────

    /// User focused an app and chose a direction. The compositor
    /// spawns the process and places the resulting window.
    ///
    /// `bin`/`args` are the SELECTED entry's, not necessarily the app's main
    /// entry — picking "New Window" launches what the desktop file declares
    /// for that action. `id` stays the app's, so usage stats aggregate per
    /// app rather than fragmenting per action.
    Launch {
        id: String,
        bin: PathBuf,
        args: Vec<String>,
        direction: Direction,
    },

    /// User dismissed the launcher. The compositor tears down the
    /// overlay surface.
    Exit,

    // ─── State actions: pure verbs the reducer applies ──────────────

    /// Move selection cursor by `delta` slots. Clamps at row ends.
    MoveCursor(i32),

    /// Move the ENTRY cursor within the focused app by `delta` — its main
    /// entry and each action its desktop file declares. Clamps at both ends.
    /// Up/Down while browsing; they were unbound there, and stay bound to
    /// spawn direction once an app is focused.
    MoveEntry(i32),

    /// Promote browse → focused. `event_process` emits this only when
    /// the visible list is non-empty.
    FocusSelection,

    /// Return from focused → browse without losing the query.
    UnfocusSelection,

    /// Empty the search query.
    ClearQuery,

    /// Drop the last character of the search query.
    Backspace,

    /// Append printable text to the query.
    AppendText(String),

    // ─── Compositor-pushed ──────────────────────────────────────────

    /// Re-rank the default list so frecency drifts as time passes.
    Tick,

    /// Replace the application list (e.g. after a desktop-file rescan).
    SetApps(Arc<Vec<crate::model::Application>>),
}