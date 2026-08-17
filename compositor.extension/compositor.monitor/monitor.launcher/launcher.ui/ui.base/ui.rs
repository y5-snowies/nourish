//! [`Launcher`] — the iced UI instance.
//!
//! ### Architecture (redux-shaped, with subscribe + event_process)
//!
//! - **`subscribe`** — bitflag of which iced event categories to
//!   observe; checked before phase 0.
//! - **`event_process(event)`** — pure `iced_event → Vec<message>`;
//!   runs in phase 0 before iced's own event dispatch.
//! - **`view`** — pure render.
//! - **`update(message)`** — pure reducer; mutates state, never re-emits.
//! - **`process(message)`** — pure follow-up derivation; unused here
//!   because `event_process` already handles every key directly.
//!
//! ### Carousel scrolling
//!
//! The view shows [`style::CAROUSEL_VISIBLE`] cells at once.
//! `scroll_offset` is the index into `visible` shown in the leftmost
//! slot. The cursor moves within `[scroll_offset, scroll_offset +
//! CAROUSEL_VISIBLE)`; pressing past either edge shifts the scroll
//! window by exactly one — like a typical edge-scrolling list.

use std::time::SystemTime;

use iced_core::keyboard::key::Named;
use iced_core::keyboard::{self, Key};
use iced_core::{Element, Event as IcedEvent, Theme};
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use compositor_support_iced_core_engine_base::ui::EventFlags;
use crate::message::LauncherMessage;
use crate::model::{Application, Direction};
use crate::{style, view};
use crate::{search};

/// Label for one entry, before any disambiguation.
///
/// An action's `Name` alone loses the app — "New Window" does not say whose —
/// so an action qualifies its app rather than replacing it.
///
/// The plain new-window action is the exception: it is the DEFAULT entry, so
/// qualifying it would put the same parenthetical on every app in the
/// carousel and say nothing. There the app name alone carries it. Recognised
/// by the same two independent indicators the desktop parser uses — the
/// action id, or its name — so a localised `Nouvelle fenêtre` is still caught
/// by its id.
fn entry_label(app: &Application, index: usize) -> String {
    let Some(entry) = app.entry(index) else { return app.title.clone() };
    let Some(action) = entry.action.as_deref() else {
        return app.title.clone(); // main entry
    };
    if action.eq_ignore_ascii_case("new-window") || entry.title.eq_ignore_ascii_case("new window") {
        return app.title.clone();
    }
    format!("{} ({})", app.title, entry.title)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Browsing,
    Focused,
}

pub struct Launcher {
    pub(crate) apps: Vec<Application>,
    pub(crate) query: String,
    pub(crate) visible: Vec<usize>,
    /// Cursor's position inside `visible`.
    pub(crate) cursor: usize,
    /// Index into `visible` shown in the leftmost carousel slot.
    /// Invariant when `visible` is non-empty:
    /// `scroll_offset <= cursor < scroll_offset + CAROUSEL_VISIBLE`
    /// (clamped to a valid sub-window of `visible`).
    pub(crate) scroll_offset: usize,
    pub(crate) mode: Mode,
    /// Which entry of the app under `cursor` is selected — its main entry or
    /// one of its declared actions. Reset to that app's `default_entry`
    /// whenever the cursor lands somewhere new, so the choice is per-visit
    /// rather than sticky across apps with unrelated action lists.
    pub(crate) entry_cursor: usize,
}

impl Launcher {
    pub fn new(apps: Vec<Application>) -> Self {
        let visible = search::rank_default(&apps, SystemTime::now());
        let mut ui = Self {
            apps,
            query: String::new(),
            visible,
            cursor: 0,
            scroll_offset: 0,
            mode: Mode::Browsing,
            entry_cursor: 0,
        };
        ui.reset_entry_cursor();
        ui
    }

    // ─── Read-only accessors used by `view` ─────────────────────────

    /// The app under the cursor, if the visible list is non-empty.
    pub(crate) fn current_app(&self) -> Option<&Application> {
        self.visible.get(self.cursor).map(|&i| &self.apps[i])
    }

    /// The selected entry of the app under the cursor.
    pub(crate) fn current_entry(&self) -> Option<&crate::model::AppEntry> {
        self.current_app()?.entry(self.entry_cursor)
    }

    /// Title shown in the footer. See [`entry_label`] for the base rule.
    pub(crate) fn current_title(&self) -> String {
        let Some(app) = self.current_app() else { return String::new() };
        let label = entry_label(app, self.entry_cursor);

        // Collapsing the new-window action to the bare app name makes it
        // collide with the MAIN entry, which renders to the same thing — so
        // moving between them changed nothing on screen. Mark the main entry
        // as the default when, and only when, that collision actually
        // happens; an app whose actions are all distinctly named needs no
        // qualifier on its main entry.
        let is_main = app.entry(self.entry_cursor).is_some_and(|e| e.action.is_none());
        let collides = (0..app.entries.len())
            .any(|i| i != self.entry_cursor && entry_label(app, i) == label);
        if is_main && collides {
            return format!("{label} (default)");
        }
        label
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.mode == Mode::Focused
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    // ─── Reducer helpers ────────────────────────────────────────────

    /// Re-run search and clamp cursor / scroll. Called whenever the
    /// query or the application list changes.
    fn recompute_visible(&mut self) {
        self.visible = search::search(&self.apps, &self.query, SystemTime::now());
        // Clamp cursor first, then re-derive a sensible scroll_offset.
        if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len().saturating_sub(1);
        }
        self.clamp_scroll();
    }

    /// Ensure the scroll window contains `cursor` and doesn't extend
    /// past the end of `visible`.
    fn clamp_scroll(&mut self) {
        let n = self.visible.len();
        if n == 0 {
            self.scroll_offset = 0;
            return;
        }
        let win = style::CAROUSEL_VISIBLE;

        // If everything fits, anchor to 0.
        if n <= win {
            self.scroll_offset = 0;
            return;
        }

        // Make sure cursor is inside the window.
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + win {
            self.scroll_offset = self.cursor + 1 - win;
        }

        // Don't let the window run off the right end.
        let max_offset = n - win;
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }
    }

    /// Move cursor by `delta`. The window scrolls only when the cursor
    /// is at one of the window's edges and would move past it.
    fn move_cursor(&mut self, delta: i32) {
        let n = self.visible.len();
        if n == 0 {
            self.cursor = 0;
            self.scroll_offset = 0;
            self.entry_cursor = 0;
            return;
        }

        let n_i = n as i32;
        let new_cursor = (self.cursor as i32 + delta).clamp(0, n_i - 1) as usize;
        self.cursor = new_cursor;
        self.clamp_scroll();
        self.reset_entry_cursor();
    }

    /// Point the entry cursor at the app's preferred entry — its declared
    /// "new window" action where it has one, else its main entry.
    fn reset_entry_cursor(&mut self) {
        self.entry_cursor = self.current_app().map(|a| a.default_entry).unwrap_or(0);
    }

    /// Move within the current app's entries. Clamped, not wrapping: the list
    /// is short and usually two long, so wrapping would make Up and Down
    /// indistinguishable.
    fn move_entry(&mut self, delta: i32) {
        let Some(count) = self.current_app().map(|a| a.entries.len()) else {
            self.entry_cursor = 0;
            return;
        };
        if count == 0 {
            self.entry_cursor = 0;
            return;
        }
        self.entry_cursor =
            (self.entry_cursor as i32 + delta).clamp(0, count as i32 - 1) as usize;
    }

    fn set_apps(&mut self, apps: Vec<Application>) {
        self.apps = apps;
        self.query.clear();
        self.mode = Mode::Browsing;
        self.cursor = 0;
        self.scroll_offset = 0;
        self.recompute_visible();
        self.reset_entry_cursor();
    }

    // ─── Event-process helpers ──────────────────────────────────────

    /// `MoveEntry` only when there is somewhere to move to.
    fn entry_step(&self, delta: i32) -> Vec<LauncherMessage> {
        match self.current_app() {
            Some(app) if app.has_choices() => vec![LauncherMessage::MoveEntry(delta)],
            _ => Vec::new(),
        }
    }

    fn decode_browsing(&self, key: &Key, text: Option<&str>) -> Vec<LauncherMessage> {
        match key {
            Key::Named(Named::ArrowLeft) => vec![LauncherMessage::MoveCursor(-1)],
            Key::Named(Named::ArrowRight) => vec![LauncherMessage::MoveCursor(1)],
            // Entry selection within the current app. Only meaningful when
            // the app declares actions, so apps with a single entry keep the
            // previous no-op behaviour rather than swallowing the key.
            Key::Named(Named::ArrowUp) => self.entry_step(-1),
            Key::Named(Named::ArrowDown) => self.entry_step(1),
            Key::Named(Named::Enter) => {
                if self.visible.is_empty() {
                    Vec::new()
                } else {
                    vec![LauncherMessage::FocusSelection]
                }
            }
            Key::Named(Named::Escape) => {
                if self.query.is_empty() {
                    vec![LauncherMessage::Exit]
                } else {
                    vec![LauncherMessage::ClearQuery]
                }
            }
            Key::Named(Named::Backspace) => {
                if self.query.is_empty() {
                    Vec::new()
                } else {
                    vec![LauncherMessage::Backspace]
                }
            }
            // IME-friendly text channel (composed key text). Using it
            // instead of parsing `Key::Character` keeps non-Latin
            // layouts and dead-key sequences working correctly.
            _ => {
                let Some(s) = text else { return Vec::new() };
                let filtered: String = s.chars().filter(|c| !c.is_control()).collect();
                if filtered.is_empty() {
                    Vec::new()
                } else {
                    vec![LauncherMessage::AppendText(filtered)]
                }
            }
        }
    }

    fn decode_focused(&self, key: &Key) -> Vec<LauncherMessage> {
        let direction = match key {
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowLeft) => Some(Direction::Left),
            Key::Named(Named::ArrowRight) => Some(Direction::Right),
            Key::Named(Named::Escape) => {
                return vec![LauncherMessage::UnfocusSelection];
            }
            _ => None,
        };

        let Some(direction) = direction else { return Vec::new() };
        let Some(app) = self.current_app() else { return Vec::new() };
        let Some(entry) = app.entry(self.entry_cursor) else { return Vec::new() };

        vec![
            LauncherMessage::Launch {
                id: app.id.clone(),
                bin: entry.bin.clone(),
                args: entry.args.clone(),
                direction,
            },
            // LauncherMessage::Exit,
        ]
    }
}

impl IcedUi for Launcher {
    type Message = LauncherMessage;

    fn subscribe(&self) -> EventFlags {
        EventFlags::KEYBOARD
    }

    fn event_process(&self, event: &IcedEvent) -> Vec<Self::Message> {
        let IcedEvent::Keyboard(keyboard::Event::KeyPressed {
            key, text, ..
        }) = event
        else {
            return Vec::new();
        };

        match self.mode {
            Mode::Browsing => self.decode_browsing(key, text.as_deref()),
            Mode::Focused => self.decode_focused(key),
        }
    }

    fn update(&mut self, message: Self::Message) {
        match message {
            LauncherMessage::Launch { .. } | LauncherMessage::Exit => {}

            LauncherMessage::MoveCursor(delta) => self.move_cursor(delta),

            LauncherMessage::MoveEntry(delta) => self.move_entry(delta),

            LauncherMessage::FocusSelection => {
                if !self.visible.is_empty() {
                    self.mode = Mode::Focused;
                }
            }

            LauncherMessage::UnfocusSelection => {
                self.mode = Mode::Browsing;
            }

            LauncherMessage::ClearQuery => {
                self.query.clear();
                self.cursor = 0;
                self.scroll_offset = 0;
                self.recompute_visible();
                self.reset_entry_cursor();
            }

            LauncherMessage::Backspace => {
                if !self.query.is_empty() {
                    self.query.pop();
                    self.cursor = 0;
                    self.scroll_offset = 0;
                    self.recompute_visible();
                    self.reset_entry_cursor();
                }
            }

            LauncherMessage::AppendText(s) => {
                if !s.is_empty() {
                    self.query.push_str(&s);
                    self.cursor = 0;
                    self.scroll_offset = 0;
                    self.recompute_visible();
                    self.reset_entry_cursor();
                }
            }

            LauncherMessage::Tick => {
                if self.query.is_empty() {
                    self.recompute_visible();
                }
            }

            LauncherMessage::SetApps(apps) => {
                let apps = std::sync::Arc::try_unwrap(apps)
                    .unwrap_or_else(|arc| (*arc).clone());
                self.set_apps(apps);
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        view::root(self)
    }
}
