//! Data types owned by the launcher.

use std::path::PathBuf;
use std::time::SystemTime;

/// One launchable entry of an application: either its main desktop entry or
/// one of the additional actions the desktop file declares under `Actions=`.
///
/// Actions are a first-class part of the Desktop Entry spec, and each carries
/// its own `Name`, `Exec` and (sometimes) `Icon` — so "Google Chrome" and
/// "New Window" are genuinely different things to launch, not one thing with
/// a modifier. Modelling them as sibling entries is what lets the launcher
/// offer them without y5 inventing flags of its own.
#[derive(Debug, Clone)]
pub struct AppEntry {
    /// Desktop action id (`new-window`), or `None` for the main entry.
    pub action: Option<String>,

    /// The action's `Name`, or the application's for the main entry.
    pub title: String,

    /// Executable to invoke.
    pub bin: PathBuf,

    /// Arguments to pass to `bin`.
    pub args: Vec<String>,

    /// Optional path to an icon file (PNG or SVG). Actions may declare their
    /// own; falls back to the application's. If `None` or the file fails to
    /// load, a glyph fallback is drawn.
    pub icon_path: Option<PathBuf>,
}

/// A launchable application.
///
/// The compositor populates a `Vec<Application>` and hands it to the
/// launcher. The launcher takes ownership; it never mutates the
/// compositor's copy.
#[derive(Debug, Clone)]
pub struct Application {
    /// Stable identifier (e.g. desktop file id). Echoed back in
    /// [`crate::message::LauncherMessage::Launch`] so the compositor
    /// can correlate.
    pub id: String,

    /// Human-readable name shown under the focused icon.
    pub title: String,

    /// Main entry first, then every action the desktop file declares, in
    /// declaration order. Never empty — an app with no launchable entry is
    /// not listed at all.
    pub entries: Vec<AppEntry>,

    /// Index into [`Application::entries`] selected when the cursor lands on
    /// this app. Points at the "new window" action where the desktop file
    /// declares one, so the common case needs no keystrokes.
    pub default_entry: usize,

    /// How many times the user has launched this app via the launcher.
    pub usage_count: u64,

    /// When the user last launched this app. `None` = never launched
    /// here.
    pub usage_time: Option<SystemTime>,
}

impl Application {
    /// The entry at `index`, clamped into range. Never panics: the cursor is
    /// clamped on every move, but the app list can be replaced underneath it.
    pub fn entry(&self, index: usize) -> Option<&AppEntry> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.get(index.min(self.entries.len() - 1))
    }

    /// Whether this app offers more than one thing to launch — the condition
    /// for showing the "more entries" affordance in the carousel.
    pub fn has_choices(&self) -> bool {
        self.entries.len() > 1
    }
}

/// Direction the user picked after focusing an icon. Emitted inside
/// [`crate::message::LauncherMessage::Launch`] so a tiling compositor
/// knows where to place the new window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
