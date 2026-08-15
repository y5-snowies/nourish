//! Overview-mode state slot (the Super+Tab overlay): a flag-gated presentational
//! overlay on the active world — window grid (Layout) / picker globe (World)
//! beneath a top menu bar, over a frozen blurred snapshot.

use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_monitor_compositor_iced_base::HandleId;
use compositor_support_bevy_core_alloc_base::AllocatedDmabuf;
use compositor_y5_graphic_capture_registry::{CaptureHandle, SnapshotHandle};
use compositor_monitor_overview_ui_hover::CardIcon;
use smithay::utils::{Physical, Rectangle};
use uuid::Uuid;

/// Height of the top menu bar (physical px); the grid reserves the same top inset.
pub const MENU_BAR_HEIGHT: i32 = 48;

/// The three menu sections. `World`/`Settings` are placeholders; `Layout` is the
/// window grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    World,
    Layout,
    Settings,
}

/// The resolved freeze backdrop: a blurred desktop copy, or the sharp snapshot.
/// `Blur` KEEPS the sharp snapshot it was derived from — the World tab reuses it
/// as the current world's globe thumbnail, and it is the only unblurred frame of
/// the pre-overlay desktop anyone still holds.
pub enum Backdrop {
    Blur(AllocatedDmabuf, SnapshotHandle),
    Sharp(SnapshotHandle),
}

/// Freeze-backdrop lifecycle (capture the desktop, then hold the blurred/sharp
/// result). `Ready(None)` = no-capture dim.
pub enum Phase {
    Closed,
    Arming { entry: CaptureHandle, countdown: u8 },
    Ready(Option<Backdrop>),
}

/// Overview-mode slot (per-world; lives on the spawn-target / active world).
pub struct Overview {
    pub visible: bool,
    pub tab: Tab,
    /// Screen-space iced menu-bar surface id (created/destroyed by the interface).
    pub menu: Option<HandleId>,
    /// The logout-confirmation popup surface id, when open (None = closed).
    pub logout: Option<HandleId>,
    /// Freeze-backdrop capture lifecycle.
    pub phase: Phase,
    /// Vertical grid scroll offset (physical px), clamped at render.
    pub scroll: f64,
    /// Last-rendered cell rects (uuid → screen rect) for click hit-testing.
    pub cells: Vec<(Uuid, Rectangle<i32, Physical>)>,
    /// The hover card surface (a click-through tooltip), created on demand while
    /// the overlay is up and destroyed with it.
    pub card: Option<HandleId>,
    /// What the card currently shows — `None` while it is blank. The title is a
    /// live read and the icon costs a theme lookup or a buffer decode, so both
    /// are pushed only when this stops matching.
    pub card_shown: Option<(Uuid, String, Option<CardIcon>)>,
}

impl Overview {
    pub fn new() -> Self {
        Self {
            visible: false,
            tab: Tab::Layout,
            menu: None,
            logout: None,
            phase: Phase::Closed,
            scroll: 0.0,
            cells: Vec::new(),
            card: None,
            card_shown: None,
        }
    }

    /// True once the freeze backdrop is resolved (snapshot taken, or the
    /// no-capture fallback) — i.e. the overlay (grid + menu) may be shown.
    pub fn overlay_ready(&self) -> bool {
        matches!(self.phase, Phase::Ready(_))
    }

    /// The frozen SHARP pre-overlay desktop frame, once resolved — i.e. the
    /// active world exactly as it looked before the overlay drew over it.
    pub fn frozen(&self) -> Option<&SnapshotHandle> {
        match &self.phase {
            Phase::Ready(Some(Backdrop::Blur(_, snap) | Backdrop::Sharp(snap))) => Some(snap),
            _ => None,
        }
    }

    pub fn is_world(&self) -> bool {
        matches!(self.tab, Tab::World)
    }

    pub fn is_settings(&self) -> bool {
        matches!(self.tab, Tab::Settings)
    }
}

/// The overview slot token (read via the core `overview()` focus accessor).
pub static OVERVIEW: Token<Overview> = Token::new();
pub static OVERVIEW_MUT: TokenMut<Overview> = TokenMut::new(&OVERVIEW);

/// The last active tab, retained ACROSS worlds (kernel store). The overview
/// slot itself stays per-world, but the tab is a session preference: every tab
/// change writes it here, and `toggle` seeds the opening world's slot from it —
/// so after entering a world via the World tab, Super+Tab reopens on World.
pub static OVERVIEW_TAB: Token<Tab> = Token::new();
pub static OVERVIEW_TAB_MUT: TokenMut<Tab> = TokenMut::new(&OVERVIEW_TAB);

/// Deferred overview action handled by the surface pump (holds the renderer).
/// `Reconcile` syncs the menu bar to `visible`; the rest come from iced message
/// handlers — `SetTab` (tab click), `ToggleLogout` (username), `Logout` (confirm).
#[derive(Debug, Clone)]
pub enum OverviewSurfaceMessage {
    Reconcile,
    SetTab(Tab),
    ToggleLogout,
    Logout,
    /// Close the overlay (the menu-bar ✕ tapped) — flips `visible` off.
    Close,
}
