//! On-screen-keyboard driver state (kernel token `OSK`). Holds the logical OSK
//! state — shown/pinned/placement, the sticky modifier toggles, the modifier
//! keycodes currently injected-down (released on close so none stick), and the
//! world-mode zoom / travel bookkeeping. The live iced surface handle itself is
//! kept in the reconciler's own thread_local (see `board.create`), mirroring the
//! touch pane. Registered once at startup (`orchestration.core` state init).
use compositor_support_system_storage_token_base::base::{Token, TokenMut};

/// Sticky modifier toggles applied to the next injected key.
#[derive(Default, Clone, Copy)]
pub struct OskMods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl OskMods {
    /// True when any modifier is toggled on.
    pub fn any(&self) -> bool {
        self.shift || self.ctrl || self.alt || self.logo
    }
}

pub struct OskState {
    /// The keyboard is shown (computed each frame by the reconciler from `pinned` +
    /// the auto-show condition; cached here so non-active output passes agree).
    pub open: bool,
    /// Pinned: stays open even when text-input focus leaves (touch-menu opened).
    pub pinned: bool,
    /// Dismissed by the Close button: suppresses AUTO-show until the focused text
    /// field goes inactive (blurs), so ✕ hides the keyboard even while a field is
    /// still focused. Cleared by the reconciler when no text-input is active.
    pub dismissed: bool,
    /// World-space placement (else the screen-space bottom bar).
    pub world: bool,
    /// Sticky modifier toggles (applied to the next injected key).
    pub mods: OskMods,
    /// Modifier keycodes (xkb = evdev+8) currently injected-DOWN, so they can be
    /// released on close — otherwise a mid-shift close would leave a stuck modifier.
    pub held: Vec<u32>,
    /// Last camera zoom the world OSK was counter-scaled for (NaN = unset).
    pub prev_zoom: f64,
    /// Camera position stashed before a screen-mode auto-show travel, restored on hide.
    pub travel_restore: Option<(f64, f64)>,
    /// Tap feedback: the keycode of the just-pressed key, highlighted for `flash_frames`
    /// frames. Input alone schedules no redraw, so the reconciler pumps frames while this
    /// is active (see `board.create`), giving a visible "pressed" flash.
    pub flash_key: Option<u32>,
    pub flash_frames: u8,
    /// Local echo of characters typed ON the OSK, shown as the preview fallback when the
    /// focused field reports no surrounding text — notably iced fields (the launcher
    /// search) and clients that don't implement it. Never populated for sensitive fields.
    pub echo: String,
    /// Identity of the field the echo belongs to (iced handle or client text-input
    /// surface, tagged). When it changes, the echo is cleared so stale text from a
    /// previous field / IME focus doesn't carry over.
    pub echo_focus: Option<u64>,
}

impl Default for OskState {
    fn default() -> Self {
        Self {
            open: false,
            pinned: false,
            dismissed: false,
            world: false,
            mods: OskMods::default(),
            held: Vec::new(),
            prev_zoom: f64::NAN,
            travel_restore: None,
            flash_key: None,
            flash_frames: 0,
            echo: String::new(),
            echo_focus: None,
        }
    }
}

pub static OSK: Token<OskState> = Token::new();
pub static OSK_MUT: TokenMut<OskState> = TokenMut::new(&OSK);
