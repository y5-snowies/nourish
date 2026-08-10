use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_monitor_compositor_iced_base::HandleId;
use compositor_model_environment_preference_base::base::{KeyBind, PenBindTarget};

/// What the settings Pen tab is currently waiting to capture (click-to-bind). Read by
/// the keyboard handler (Key) and the libinput pad handler (Pad); armed/cleared via
/// the settings handler; the captured result is drained to the UI by the reconciler.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum PenCapture {
    #[default]
    None,
    /// Capture the next keyboard combo (bind a key to a pad/stylus button).
    Key,
    /// Capture the next pad button press (identify a pad button to bind).
    Pad,
}

/// Settings-window driver data: the live screen-space iced surface (opened with
/// Super+. , closed from its button) plus UI lifecycle flags. `open` is the
/// desired state the keybinding toggles; the render-path reconciler in
/// `settings.interface` creates/destroys the surface to match (it needs the
/// GlesRenderer, available only on the draw path). `dirty` records that an
/// Environment (settings.json) field changed this session — those are read once
/// at startup, so the window shows a "reboot to apply" banner. Live preferences
/// (cursor speed, natural-scroll, output modes) apply immediately and never set it.
#[derive(Default)]
pub struct SettingsState {
    pub open: bool,
    pub handle: Option<HandleId>,
    pub dirty: bool,
    /// True only while the Performance tab is the visible settings module — the
    /// gate for pushing live FPS (so other tabs don't buffer per-frame updates).
    pub fps_wanted: bool,
    /// Last selected settings module, kept across reopens for the session so the
    /// panel restores the tab the user left on. Stored as the `Tab::to_index`
    /// value (orchestration can't name the configurator `Tab`); 0 = Display.
    pub tab: u8,
    /// Last selected shader-picker category, kept for the same reason and in the
    /// same place: reopening onto the World tab should land where it was left,
    /// and the picker is long enough that "All" every time means scrolling back
    /// to the same group. `None` = All.
    pub shader_category: Option<String>,
    /// Pen-tab click-to-bind capture: what we're waiting for (armed by the settings
    /// handler), the control a captured key binds to, and the raw captured result
    /// (the reconciler applies it to the live pen config, persists, and syncs the UI).
    pub pen_capture: PenCapture,
    pub pen_capture_target: Option<PenBindTarget>,
    pub pen_captured_key: Option<KeyBind>,
    pub pen_captured_pad: Option<(String, u32)>,
}

pub static SETTINGS: Token<SettingsState> = Token::new();
pub static SETTINGS_MUT: TokenMut<SettingsState> = TokenMut::new(&SETTINGS);
