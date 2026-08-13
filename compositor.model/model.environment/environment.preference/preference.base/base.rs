//! Live user **preferences** — the inline-reloaded counterpart to the read-once
//! `environment.config` (settings.json). Stored in
//! `~/.config/y5.compositor/preferences.json` (same dir). Loaded fresh ([`load`])
//! rather than cached in a startup `OnceLock`: startup seeds the runtime cells,
//! the settings window re-reads on open (so terminal edits show) and writes back
//! with [`save`]; applied live, no reboot.

use compositor_model_environment_background_base::base::TripleBufferBackground;
use compositor_model_environment_interface_base::base::TripleBufferUI;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A per-output mode preference keyed by EDID identity. `Advertised` is the only
/// variant applied by default policy; the synthesis variants require the separate
/// mode-synthesis safety enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModeRequest {
    /// Pick from the modes the monitor advertises.
    Advertised { width: u16, height: u16, refresh_mhz: u32 },
    /// Synthesize via CVT (requires the mode-synthesis safety enable).
    Cvt { width: u16, height: u16, refresh: f64 },
    /// Raw modeline string (requires the mode-synthesis safety enable).
    Modeline(String),
}

fn profile_active_default() -> bool {
    true
}

/// Default for `release_hidden_surfaces`: on. An older `preferences.json` without
/// the field gets the memory-saving behavior by default.
fn default_release_hidden() -> bool {
    true
}

/// Default for `fractional_invisible`: `"full"` — invisible windows are frozen
/// and published scale 1 (with the grace band pre-publishing real scales near
/// pane edges), so both fresh installs and older `preferences.json` files get
/// the resource-saving behavior unless explicitly set back to "optimized"/"off".
/// Default for `input_edge_pan_continuous`: on. An older `preferences.json` gets
/// the RTS-style behaviour, which is what the edge pan is expected to feel like.
fn default_edge_pan_continuous() -> bool {
    true
}

fn default_fractional_invisible() -> String {
    "full".to_string()
}

/// Per-monitor output preference. `identity = None` applies to any output
/// (single-output-era default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputProfile {
    /// EDID identity string ("make model serial") this profile applies to.
    pub identity: Option<String>,
    pub mode: Option<ModeRequest>,
    /// Whether this monitor is DRIVEN. `false` = the user deactivated it (settings
    /// Display tab → "Inactive"): the compositor doesn't light it and it's dropped
    /// from the live cursor-teleport map, but its profile + map placement are kept
    /// so reactivating restores it in place. Defaults to `true` (all monitors active),
    /// so an older `preferences.json` without the field drives every monitor as before.
    #[serde(default = "profile_active_default")]
    pub active: bool,
    /// Stable id ("name serial") of the touch INPUT device the user claimed for
    /// this monitor in the settings Display tab, so its touches route here. A device
    /// is claimed by at most one monitor. `None` = auto-correlated (size/EDID/USB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch_device: Option<String>,
}

impl Default for OutputProfile {
    fn default() -> Self {
        Self { identity: None, mode: None, active: true, touch_device: None }
    }
}

/// One placed monitor in the cursor-teleport layout (the settings Display-tab
/// canvas). Purely a cursor-crossing map: `x`/`y`/`size` are abstract layout-space
/// coordinates (a unit-agnostic arrangement grid), NOT physical pixels, and never
/// affect a monitor's scale or resolution. `identity` is the EDID key ("make model
/// serial"); the SAME identity may appear in several placements (each an extra
/// teleport zone for that monitor). Squares are kept square (`size` = side length).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutPlacement {
    /// Stable per-placement id (disambiguates duplicate placements of one monitor).
    pub id: u64,
    /// EDID identity ("make model serial") of the monitor this square represents.
    pub identity: String,
    /// Top-left in abstract layout space.
    pub x: f32,
    pub y: f32,
    /// Width and height of the teleport zone in abstract layout space — a free
    /// rectangle (not constrained to a square). `#[serde(default)]` so a layout saved
    /// by the older square format (a `size` key, no `w`/`h`) still loads, defaulting
    /// the extents (its stored position is kept).
    #[serde(default = "layout_extent_default")]
    pub w: f32,
    #[serde(default = "layout_extent_default")]
    pub h: f32,
}

/// Default extent for a teleport-layout placement (matches the UI's `PLACE_SIZE`).
fn layout_extent_default() -> f32 {
    120.0
}

/// Fallback mode for any monitor that has no per-output [`OutputProfile`] mode yet.
/// Manually set in preferences.json only (no UI). `refresh_mhz` is mHz (e.g.
/// `60000` = 60 Hz) to match [`ModeRequest::Advertised`]; an implausibly small
/// value is normalized to 30 Hz on [`load`] (see `MIN_DEFAULT_REFRESH_MHZ`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DefaultMode {
    pub width: u16,
    pub height: u16,
    pub refresh_mhz: u32,
}

/// The complete preferences document. `#[serde(default)]` so a partial or older
/// `preferences.json` (or a missing file) still loads with sane per-field values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preference {
    /// Pointer cursor speed multiplier on relative motion (`1.0` = unscaled).
    pub cursor_sensitivity: f64,
    /// Natural scrolling: invert the touchpad finger-axis direction for canvas
    /// pan, window scroll, and multi-finger swipe navigation (wheel unaffected).
    pub input_natural_scroll: bool,
    /// Edge pan: pushing the cursor past a screen extent PANS the canvas instead of
    /// just pinning the cursor there — including while a canvas grab (move / scale /
    /// hand) is in progress, but never during a select box. While this is on, the
    /// cursor only crosses to another monitor (the `outputs_layout` teleport map)
    /// when Super is held with no canvas grab active. Off = the historical
    /// behaviour: an extent teleports when a monitor is placed across it, else
    /// clamps. Settings → Input → Mouse & Touchpad; read live per motion event.
    pub input_edge_pan: bool,
    /// Edge-pan speed: a multiplier ON TOP of the pointer speed, so the canvas
    /// travels at `cursor_sensitivity × this` — the edge pan stays proportional to
    /// how fast the pointer itself moves, and this only re-weights it. `1.0` = the
    /// push is carried into the canvas 1:1. Settings → Input → Mouse & Touchpad;
    /// read live per motion event / per frame.
    pub input_edge_pan_speed: f64,
    /// Continuous edge pan (RTS-style), ON by default: a cursor parked against a
    /// screen extent keeps the canvas moving without having to be pushed again, and
    /// pushing INTO the edge adds its own travel on top for as long as the push
    /// lasts. Off = the pan only advances while the pointer is actually pushing.
    /// An absolute pointer (winit without a locked cursor) has no push to give, so
    /// its edge band is continuous either way. Settings → Input → Mouse & Touchpad.
    #[serde(default = "default_edge_pan_continuous")]
    pub input_edge_pan_continuous: bool,
    /// Touch pan speed: a multiplier on finger-driven canvas pans (both the
    /// 2-finger pan and the single-finger glide). `1.0` = the built-in default
    /// gain; lower is slower. Touch only — the trackpad/mouse axis is unaffected.
    /// Read live per touch event (Settings → Input → Touch).
    pub input_touch_pan_speed: f64,
    /// Linear (strict) touch pan: a finger pan tracks 1:1 with NO post-release
    /// momentum/coast. On by default. Off restores the glide/fling feel. Read live
    /// per touch event (Settings → Input → Touch).
    pub input_touch_linear_pan: bool,
    /// On-screen keyboard size multiplier (`1.0` = default). Scales the OSK's height
    /// as a fraction of the output. Settings → Input → Touch. Read live by the OSK.
    pub osk_size: f64,
    /// Auto-summoned OSK floats in WORLD space near the caret (constrained, scales
    /// with zoom, like an IME) instead of the screen-space bottom bar. The touch-menu
    /// OSK is always screen-space. Settings → Input → Touch. Read live.
    pub osk_world_position: bool,
    /// Show the per-monitor FPS overlay (Settings → Performance). Off by default;
    /// each driven output gets a small top-right counter of its own draw rate.
    #[serde(default)]
    pub show_fps: bool,
    /// Release the GPU backing (dmabuf) of iced surfaces that have been hidden
    /// (off-screen or fully obstructed) for a while, re-allocating on reveal.
    /// On by default (Settings → Performance); off keeps every surface resident.
    #[serde(default = "default_release_hidden")]
    pub release_hidden_surfaces: bool,
    /// Fractional-scale strategy for INVISIBLE windows — off-screen in every
    /// viewport pane or parked in a non-hosted world (Settings → Performance;
    /// read live per frame). `"off"` = invisible windows keep receiving
    /// zoom-driven scale updates (the historical behavior). `"optimized"` =
    /// invisible windows get no publishes until visible again; other worlds'
    /// windows are published scale 1 so their clients drop hi-res buffers.
    /// `"full"` = the hosted world's invisible windows are published scale 1
    /// too. The real scale re-publishes on reveal. Capture targets always
    /// count as visible and keep updating.
    #[serde(default = "default_fractional_invisible")]
    pub fractional_invisible: String,
    /// Background triple buffering: run the background shader on its own thread
    /// and `VkDevice`, into three dmabufs the compositor samples. Vulkan only —
    /// the GLES path never takes it. Applied live; only `enabled` needs a restart.
    #[serde(default)]
    pub background_triple_buffer: TripleBufferBackground,
    /// UI triple buffering: give the iced and bevy surfaces a ring of dmabufs
    /// instead of the single one the compositor samples in the same frame it was
    /// written, and on Vulkan run bevy's scenes on their own thread. `slots` and
    /// `pipeline` apply live; `enabled` needs a restart, since it also decides
    /// whether the worker thread exists. Opt-in (default off) until it has run on
    /// real hardware.
    #[serde(default)]
    pub interface_triple_buffer: TripleBufferUI,
    /// Per-output mode preferences, priority-ordered: the FIRST entry is the
    /// default/preferred output (see `display.base`'s `profiles.first()`).
    pub outputs: Vec<OutputProfile>,
    /// Fallback mode for monitors without a per-output profile (manual only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs_default_mode: Option<DefaultMode>,
    /// The cursor-teleport layout: squares placed on the settings Display-tab
    /// canvas. Empty (the default) = single-monitor / no custom teleport, so the
    /// pointer clamps to its output exactly as before. Many-per-identity (unlike
    /// `outputs`, which is one-per-identity), so it is its own list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs_layout: Vec<LayoutPlacement>,
    /// Cursor-teleport CYCLIC mode: when the pointer exits a layout edge with no
    /// monitor across it, wrap around and re-enter from the opposite side of the
    /// layout (toroidal), instead of clamping. Default `false` (clamp at the edge).
    #[serde(default)]
    pub teleport_cyclic: bool,
    /// The input method the compositor launches at startup. y5 spawns exactly this
    /// process and grants the input-method / virtual-keyboard globals (system-wide
    /// input power) ONLY to that process group — so identity is the spawned pid, never
    /// a guessed `/proc` match. When unset (or `exec` empty), y5 launches no input
    /// method — there is no built-in default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ime: Option<Ime>,
    /// Keyboard layout (xkb). Applied live on change and at startup. Defaults to
    /// `Env` so an existing `preferences.json` (no `keyboard` key) behaves exactly
    /// as before — libxkbcommon reads the `XKB_DEFAULT_*` environment.
    pub keyboard: KeyboardLayout,
    /// Default background shader for new worlds: a bundle folder name under
    /// `~/.local/share/y5/background/shader/`, or an absolute path. A world may
    /// override it in its own record; unset = the built-in parallax.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_shader: Option<String>,
    /// Anti-aliasing / graphics config for the pannable world (edited in the
    /// settings "Graphics" tab). Applied live and pushed to the kernel renderer.
    #[serde(default)]
    pub graphics: compositor_model_environment_graphics_base::base::GraphicsAaConfig,
    /// Page-flip policy: tearing + its pacing fallback (settings "Performance").
    /// Lives here rather than settings.json so it applies live — the scanout
    /// layer reads the mirrored global per frame.
    ///
    /// Deliberately a NEW key rather than reusing `tearing`: the old value has an
    /// incompatible shape, and serde's field-level default only covers a MISSING
    /// key, not a malformed one. Reusing the name would fail the whole document
    /// and silently reset every unrelated preference. Unknown keys are ignored
    /// (no `deny_unknown_fields`), so the stale `tearing` entry simply lapses.
    #[serde(default)]
    pub flip: compositor_model_environment_tearing_config::config::Config,
    /// wlr + ext foreign-toplevel-management (taskbar/dock protocols), gated as ONE
    /// preference: `"enabled"` advertises open windows to docks; anything else
    /// (the default) keeps both globals bound but mute (no toplevels announced,
    /// control requests ignored). Read once at startup — a change takes effect on
    /// the next launch. Edited in the Misc tab.
    #[serde(default = "default_protocol_foreign")]
    pub protocol_foreign: String,
    /// When true, the foreign-toplevel advertisement shows windows from EVERY world,
    /// not just the hosted (active) one. Startup snapshot, like `protocol_foreign`.
    /// Edited on the same (Misc) tab.
    #[serde(default)]
    pub protocol_foreign_all_worlds: bool,
    /// Pen / tablet overrides (Settings → Pen). Applied live. Empty by default, so a
    /// stock tablet keeps its driver defaults until the user binds something.
    #[serde(default)]
    pub pen: PenConfig,
}

/// Default for `protocol_foreign`: `"disabled"` — off unless the user opts in, so
/// an older `preferences.json` (or a fresh install) does not expose the dock
/// protocols by default.
fn default_protocol_foreign() -> String {
    "disabled".to_string()
}

/// Where the keyboard layout comes from. `Env` (the historical default) leaves the
/// xkb config empty so libxkbcommon reads the `XKB_DEFAULT_*` environment variables;
/// `Manual` uses the explicit [`KeyboardLayout`] fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutSource {
    /// Read layout/variant/options from the `XKB_DEFAULT_*` environment.
    Env,
    /// Use the explicit `layout`/`variant`/`options` fields.
    Manual,
}

/// Keyboard layout (xkb) preference. Applied live on change and at startup via
/// `compositor_support_smithay_state_seat_xkb`. `Manual` uses the ordered
/// [`layouts`](KeyboardLayout::layouts) list (the first is the default; the
/// [`switch`](KeyboardLayout::switch) hotkey cycles them). Per-layout variants and
/// arbitrary xkb options are intentionally NOT configurable here — set them through
/// the `XKB_DEFAULT_*` environment (the `Env` source). The legacy `layout`/`variant`/
/// `options` scratch fields are read only to migrate an old file (see [`normalize`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardLayout {
    pub source: LayoutSource,
    /// Ordered xkb layout codes, e.g. `["us", "il"]`. First = default; the switch
    /// hotkey cycles through them in order. Empty falls back to the xkb default (us).
    pub layouts: Vec<String>,
    /// Preset layout-switch hotkey (maps to an xkb `grp:` option). Only meaningful
    /// with two or more layouts.
    pub switch: LayoutSwitch,

    // --- Legacy single-layout fields (pre-multi-layout). Read once on load to
    // migrate into `layouts`/`switch`, then cleared; never re-serialized. ---
    #[serde(default, skip_serializing)]
    pub layout: String,
    #[serde(default, skip_serializing)]
    pub variant: String,
    #[serde(default, skip_serializing)]
    pub options: String,
}

impl Default for KeyboardLayout {
    fn default() -> Self {
        Self {
            source: LayoutSource::Env,
            // Empty (not `["us"]`) so a MISSING `layouts` key is distinguishable from
            // an explicit choice — `normalize` migrates a legacy `layout` into it.
            layouts: Vec::new(),
            switch: LayoutSwitch::None,
            layout: String::new(),
            variant: String::new(),
            options: String::new(),
        }
    }
}

/// A preset layout-switch hotkey. Maps to the xkb `grp:` option that libxkbcommon
/// uses to cycle the ordered layout list. Arbitrary xkb options are not exposed —
/// this closed set keeps the config crash-safe and the UI a simple dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LayoutSwitch {
    /// No switch key (single layout, or switching handled elsewhere).
    #[default]
    None,
    AltShift,
    CtrlShift,
    AltSpace,
    CtrlSpace,
    SuperSpace,
    CapsToggle,
}

impl LayoutSwitch {
    /// Every variant, in dropdown order.
    pub const ALL: &'static [LayoutSwitch] = &[
        LayoutSwitch::None,
        LayoutSwitch::AltShift,
        LayoutSwitch::CtrlShift,
        LayoutSwitch::AltSpace,
        LayoutSwitch::CtrlSpace,
        LayoutSwitch::SuperSpace,
        LayoutSwitch::CapsToggle,
    ];

    /// Human-readable label for the settings dropdown.
    pub fn label(self) -> &'static str {
        match self {
            LayoutSwitch::None => "None",
            LayoutSwitch::AltShift => "Alt + Shift",
            LayoutSwitch::CtrlShift => "Ctrl + Shift",
            LayoutSwitch::AltSpace => "Alt + Space",
            LayoutSwitch::CtrlSpace => "Ctrl + Space",
            LayoutSwitch::SuperSpace => "Super + Space",
            LayoutSwitch::CapsToggle => "Caps Lock",
        }
    }

    /// The xkb `grp:` option string, or `None` for [`LayoutSwitch::None`].
    pub fn grp_option(self) -> Option<&'static str> {
        match self {
            LayoutSwitch::None => None,
            LayoutSwitch::AltShift => Some("grp:alt_shift_toggle"),
            LayoutSwitch::CtrlShift => Some("grp:ctrl_shift_toggle"),
            LayoutSwitch::AltSpace => Some("grp:alt_space_toggle"),
            LayoutSwitch::CtrlSpace => Some("grp:ctrl_space_toggle"),
            LayoutSwitch::SuperSpace => Some("grp:win_space_toggle"),
            LayoutSwitch::CapsToggle => Some("grp:caps_toggle"),
        }
    }

    /// Recover a preset from a legacy free-text `options` string (migration only):
    /// the first known `grp:` token wins; anything else is dropped.
    fn from_legacy_options(options: &str) -> LayoutSwitch {
        LayoutSwitch::ALL
            .iter()
            .copied()
            .find(|s| s.grp_option().is_some_and(|g| options.split(',').any(|tok| tok.trim() == g)))
            .unwrap_or(LayoutSwitch::None)
    }
}

// ── Pen / tablet configuration (Settings → Pen) ────────────────────────────────
// Most tablets ship no Linux configurator, so y5 lets the user override the driver
// defaults per control. Every override is opt-in: an absent binding (or
// `PenAction::Passthrough`) keeps the native driver / tablet-v2 behavior, so a
// stock-configured tablet is untouched.

/// A pointer button a pen/pad control can emulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PenButton {
    Left,
    Right,
    Middle,
}

impl PenButton {
    /// evdev button code (`<linux/input-event-codes.h>`).
    pub fn code(self) -> u32 {
        match self {
            PenButton::Left => 0x110,
            PenButton::Right => 0x111,
            PenButton::Middle => 0x112,
        }
    }
}

/// A captured keyboard combo, stored as evdev keycodes so injection replays it
/// without an xkb keysym lookup. `hold = true` presses on the trigger's press and
/// releases on its release (a held chord); `false` fires a full press+release tap.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KeyBind {
    /// Modifier keycodes held around `key` (e.g. LEFTALT=56), pressed first.
    pub mods: Vec<u32>,
    /// The main evdev keycode.
    pub key: u32,
    pub hold: bool,
}

/// What a pad button, stylus button, or the dial is remapped to. `Passthrough`
/// (the default for every control) keeps the native tablet-v2 / driver behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PenAction {
    /// Keep native behavior (forward the protocol event / driver default).
    Passthrough,
    /// Toggle the canvas Hand grab (navigation mode).
    ToggleHandMode,
    /// Open the touch pane ("touch menu").
    OpenTouchMenu,
    /// Emulate a pointer button.
    Click(PenButton),
    /// Inject a keyboard combo into the focused client.
    Key(KeyBind),
    /// Dial only: zoom the canvas (independent of hand mode).
    Zoom,
    /// Dial only: emit a scroll wheel, optionally with held modifier keycodes — e.g.
    /// `Alt`+Wheel for apps without tablet-v2 (brush size). Empty `mods` = plain wheel.
    Wheel { mods: Vec<u32> },
}

impl Default for PenAction {
    fn default() -> Self {
        PenAction::Passthrough
    }
}

/// Per-tablet overrides of the driver defaults. Serialized in `preferences.json`;
/// `#[serde(default)]` keeps older files loading. Maps are keyed by stringified
/// codes (JSON object keys) and are empty by default (⇒ every control passes through).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PenConfig {
    /// Pad-button overrides, keyed `"<device-sysname>\u{1f}<button-index>"`.
    pub pad_buttons: HashMap<String, PenAction>,
    /// Stylus barrel-button overrides, keyed by evdev button code as text. Absent ⇒
    /// the built-in default (lower barrel → right-click, upper → middle-click).
    pub stylus_buttons: HashMap<String, PenAction>,
    /// Dial-turn override. `Passthrough` still zooms while the Hand grab is held.
    pub dial: PenAction,
    /// "Pressure pen-down" mode: derive the pen-down from `tip_threshold` and IGNORE
    /// the driver's own tip event — for tablets that report "pen down" on mere
    /// detection / max distance. Below the threshold the pen hovers (a cursor, plus the
    /// bound barrel clicks); at/above it, a real press (draw / click). Off by default.
    pub below_threshold_cursor: bool,
    /// Pressure (0..1) at/above which the pen counts as "down" in pressure pen-down
    /// mode. Raise it so a light hover (which a bad driver may report as a press)
    /// doesn't register; lower it for a hair trigger.
    pub tip_threshold: f32,
    /// Stylus button (evdev code) that left-clicks while below the threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below_left: Option<u32>,
    /// Stylus button (evdev code) that right-clicks while below the threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below_right: Option<u32>,
}

impl PenConfig {
    /// U+001F unit separator joining device + button in a `pad_buttons` key (neither
    /// a sysname nor a decimal button index contains it).
    pub fn pad_key(device: &str, button: u32) -> String {
        format!("{device}\u{1f}{button}")
    }

    /// The action bound to a pad button, or `Passthrough` if unbound.
    pub fn pad_action(&self, device: &str, button: u32) -> PenAction {
        self.pad_buttons
            .get(&Self::pad_key(device, button))
            .cloned()
            .unwrap_or(PenAction::Passthrough)
    }

    /// The action bound to a stylus barrel button, or `Passthrough` if unbound.
    pub fn stylus_action(&self, button: u32) -> PenAction {
        self.stylus_buttons
            .get(&button.to_string())
            .cloned()
            .unwrap_or(PenAction::Passthrough)
    }

    /// Bind a captured key combo to `target` (Settings → Pen click-to-bind).
    pub fn set_key_bind(&mut self, target: &PenBindTarget, bind: KeyBind) {
        let action = PenAction::Key(bind);
        match target {
            PenBindTarget::Stylus(code) => {
                self.stylus_buttons.insert(code.to_string(), action);
            }
            PenBindTarget::Pad { device, button } => {
                self.pad_buttons.insert(Self::pad_key(device, *button), action);
            }
        }
    }

    /// Register a captured pad button (default action) so it appears as a bindable row.
    pub fn add_pad_button(&mut self, device: &str, button: u32) {
        self.pad_buttons
            .entry(Self::pad_key(device, button))
            .or_insert(PenAction::OpenTouchMenu);
    }
}

/// Which pen control a captured key combo binds to (Settings → Pen click-to-bind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PenBindTarget {
    /// A stylus barrel button, by evdev code.
    Stylus(u32),
    /// A pad button, by device sysname + button index.
    Pad { device: String, button: u32 },
}

/// The input-method program y5 launches, e.g. `{ "exec": "fcitx5", "args": ["-r"] }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ime {
    /// Executable to run (looked up on `PATH`). Empty = launch nothing.
    pub exec: String,
    /// Arguments passed to `exec`. Do NOT pass a daemonizing flag (`-d`): y5 must keep
    /// the process as a direct child to know its pid/process-group.
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for Preference {
    fn default() -> Self {
        Self {
            cursor_sensitivity: 1.0,
            input_natural_scroll: true,
            input_edge_pan: false,
            input_edge_pan_speed: 1.0,
            input_edge_pan_continuous: default_edge_pan_continuous(),
            input_touch_pan_speed: 1.0,
            input_touch_linear_pan: true,
            osk_size: 1.0,
            osk_world_position: false,
            show_fps: false,
            release_hidden_surfaces: true,
            fractional_invisible: default_fractional_invisible(),
            background_triple_buffer: TripleBufferBackground::default(),
            interface_triple_buffer: TripleBufferUI::default(),
            outputs: Vec::new(),
            outputs_default_mode: None,
            outputs_layout: Vec::new(),
            teleport_cyclic: false,
            ime: None,
            keyboard: KeyboardLayout::default(),
            background_shader: None,
            graphics: compositor_model_environment_graphics_base::base::GraphicsAaConfig::default(),
            flip: compositor_model_environment_tearing_config::config::Config::default(),
            protocol_foreign: default_protocol_foreign(),
            protocol_foreign_all_worlds: false,
            pen: PenConfig::default(),
        }
    }
}

/// Lowest plausible refresh in mHz. A `outputs_default_mode.refresh_mhz` below this
/// (someone typed `60` meaning 60 Hz, or a nonsense value) is normalized to 30 Hz.
const MIN_DEFAULT_REFRESH_MHZ: u32 = 20_000;

/// Sanitize a freshly-loaded document: clamp an implausible default-mode refresh
/// up to 30 Hz so a hand-edited file can't drive a monitor at a garbage rate, and
/// migrate a legacy single-layout keyboard preference into the ordered list.
pub fn normalize(mut p: Preference) -> Preference {
    // Clamp the ring depth in the struct itself, not only where it is published,
    // so the settings UI edits an already-valid value and a save round-trips it.
    p.interface_triple_buffer = p.interface_triple_buffer.normalized();
    if let Some(m) = p.outputs_default_mode.as_mut() {
        if m.refresh_mhz < MIN_DEFAULT_REFRESH_MHZ {
            m.refresh_mhz = 30_000;
        }
    }
    // Keep a hand-edited pan speed in a sane range (a 0 would freeze touch pan,
    // a huge value would fling the world off-screen).
    if !p.input_touch_pan_speed.is_finite() || p.input_touch_pan_speed <= 0.0 {
        p.input_touch_pan_speed = 1.0;
    }
    p.input_touch_pan_speed = p.input_touch_pan_speed.clamp(0.1, 4.0);
    if !p.osk_size.is_finite() || p.osk_size <= 0.0 {
        p.osk_size = 1.0;
    }
    p.osk_size = p.osk_size.clamp(0.6, 1.4);
    // Keyboard migration: an old file has `layout`/`variant`/`options` but no
    // `layouts`/`switch`. Seed the ordered list from the comma-separated `layout`
    // and recover a preset switch from a known `grp:` option, then clear the legacy
    // scratch (it is `skip_serializing`, so it never round-trips again).
    let k = &mut p.keyboard;
    if k.layouts.is_empty() && !k.layout.is_empty() {
        k.layouts = k.layout.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    if k.switch == LayoutSwitch::None && !k.options.is_empty() {
        k.switch = LayoutSwitch::from_legacy_options(&k.options);
    }
    k.layout.clear();
    k.variant.clear();
    k.options.clear();
    // Keep a hand-edited pen threshold in the valid pressure range.
    if !p.pen.tip_threshold.is_finite() {
        p.pen.tip_threshold = 0.0;
    }
    p.pen.tip_threshold = p.pen.tip_threshold.clamp(0.0, 1.0);
    p.flip = p.flip.normalized();
    p
}

/// `preferences.json`, in the same config dir as `settings.json` (honoring
/// `--config-file`/`XDG_CONFIG_HOME` via the shared resolver).
fn path() -> PathBuf {
    compositor_model_environment_config_base::base::resolve_path()
        .with_file_name("preferences.json")
}

/// Load the preferences fresh from disk. A missing or invalid file yields the
/// defaults (so the compositor and the settings window always have sane values).
pub fn load() -> Preference {
    // Falling back to defaults on a parse failure is DELIBERATE — including
    // background triple buffering, which defaults on. A file we cannot read gets
    // the same treatment as a machine that has none.
    //
    // The log is the point: without it, an unreadable file looks identical to one
    // whose settings simply had no effect, and every value in it vanishes with no
    // trace of why.
    let prefs = match std::fs::read_to_string(path()) {
        Err(_) => Preference::default(),
        Ok(raw) => match serde_json::from_str::<Preference>(&raw) {
            Ok(p) => normalize(p),
            Err(e) => {
                error!(
                    "preferences.json failed to parse ({e}); every setting in it is \
                     ignored and defaults used instead. Fix the file at {}",
                    path().display()
                );
                Preference::default()
            }
        },
    };
    // Mirror the graphics config into the kernel-readable global.
    compositor_model_environment_graphics_base::base::set(prefs.graphics);
    compositor_model_environment_tearing_config::config::set(prefs.flip);
    // Background triple buffering: the worker re-reads this every pass, so
    // publishing here is what makes the knobs apply live. `enabled` still needs a
    // restart — it decides whether the worker thread and its device exist at all.
    compositor_model_environment_background_base::base::set(
        prefs.background_triple_buffer.normalized(),
    );
    // UI triple buffering: the iced/bevy surfaces re-read this each frame and
    // grow or collapse their ring to match, so `slots` and `pipeline` apply live.
    // `enabled` does not: on Vulkan it also decides whether bevy's worker thread
    // exists, and instances are bound to the backend they were created on.
    compositor_model_environment_interface_base::base::set(
        prefs.interface_triple_buffer.normalized(),
    );
    prefs
}

/// Persist `prefs` atomically (write to a sibling `.tmp`, then rename over the
/// target — a partial write can never replace a good file).
pub fn save(prefs: &Preference) -> Result<(), String> {
    // Keep the kernel-readable global in sync with every live edit.
    compositor_model_environment_graphics_base::base::set(prefs.graphics);
    compositor_model_environment_tearing_config::config::set(prefs.flip);
    // Background triple buffering: the worker re-reads this every pass, so
    // publishing here is what makes the knobs apply live. `enabled` still needs a
    // restart — it decides whether the worker thread and its device exist at all.
    compositor_model_environment_background_base::base::set(
        prefs.background_triple_buffer.normalized(),
    );
    // UI triple buffering: the iced/bevy surfaces re-read this each frame and
    // grow or collapse their ring to match, so even `enabled` applies live here —
    // unlike the background, there is no worker thread whose existence it decides.
    compositor_model_environment_interface_base::base::set(
        prefs.interface_triple_buffer.normalized(),
    );
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let json =
        serde_json::to_string_pretty(prefs).map_err(|e| format!("serialize preferences: {e}"))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("rename {}: {e}", p.display()))?;
    Ok(())
}

/// Set the mode for the output profile matching `edid_key`, inserting a new
/// profile if none exists. Other fields of an existing profile are preserved.
pub fn upsert_output(outputs: &mut Vec<OutputProfile>, edid_key: &str, mode: ModeRequest) {
    if let Some(p) = outputs.iter_mut().find(|p| p.identity.as_deref() == Some(edid_key)) {
        p.mode = Some(mode);
    } else {
        outputs.push(OutputProfile { identity: Some(edid_key.to_string()), mode: Some(mode), active: true, touch_device: None });
    }
}

/// Whether the monitor keyed by `edid_key` is DRIVEN (active). Unknown monitors —
/// and any without an explicit profile — default to active.
pub fn output_active(outputs: &[OutputProfile], edid_key: &str) -> bool {
    outputs
        .iter()
        .find(|p| p.identity.as_deref() == Some(edid_key))
        .map(|p| p.active)
        .unwrap_or(true)
}

/// Set the active (driven) flag for `edid_key`, inserting an identity-only profile
/// if none exists. Preserves the profile's mode + position.
pub fn set_active(outputs: &mut Vec<OutputProfile>, edid_key: &str, active: bool) {
    if let Some(p) = outputs.iter_mut().find(|p| p.identity.as_deref() == Some(edid_key)) {
        p.active = active;
    } else {
        outputs.push(OutputProfile { identity: Some(edid_key.to_string()), mode: None, active, touch_device: None });
    }
}

/// The touch-device id (if any) claimed by the monitor keyed by `edid_key`.
pub fn touch_device_of<'a>(outputs: &'a [OutputProfile], edid_key: &str) -> Option<&'a str> {
    outputs
        .iter()
        .find(|p| p.identity.as_deref() == Some(edid_key))
        .and_then(|p| p.touch_device.as_deref())
}

/// The monitor (EDID key) that claimed touch device `device_id`, if any.
pub fn output_for_touch(outputs: &[OutputProfile], device_id: &str) -> Option<String> {
    outputs
        .iter()
        .find(|p| p.touch_device.as_deref() == Some(device_id))
        .and_then(|p| p.identity.clone())
}

/// Claim (`Some`) or release (`None`) touch device `device_id` for `edid_key`. A
/// device belongs to at most one monitor, so a claim first clears that id from
/// EVERY other profile, then sets it on the target (inserting an identity-only
/// profile if none exists). Preserves the target's mode/active/position.
pub fn set_touch_device(outputs: &mut Vec<OutputProfile>, edid_key: &str, device_id: Option<String>) {
    if let Some(id) = &device_id {
        for p in outputs.iter_mut() {
            if p.touch_device.as_deref() == Some(id.as_str()) {
                p.touch_device = None;
            }
        }
    }
    if let Some(p) = outputs.iter_mut().find(|p| p.identity.as_deref() == Some(edid_key)) {
        p.touch_device = device_id;
    } else if device_id.is_some() {
        outputs.push(OutputProfile {
            identity: Some(edid_key.to_string()),
            mode: None,
            active: true,
            touch_device: device_id,
        });
    }
}

/// Replace the whole cursor-teleport layout (the settings canvas commits the full
/// arrangement at once on drag-end, so there is no per-square upsert).
pub fn set_layout(prefs: &mut Preference, placements: Vec<LayoutPlacement>) {
    prefs.outputs_layout = placements;
}

/// Make the profile for `edid_key` the FIRST entry in `outputs` — the default /
/// preferred output the compositor drives (`display.base` uses `profiles.first()`).
/// Reuses an existing profile (preserving its mode) or creates an identity-only one,
/// then moves it to the front. Shared by the settings window and the settings editor.
pub fn set_default(outputs: &mut Vec<OutputProfile>, edid_key: &str) {
    let profile = match outputs.iter().position(|p| p.identity.as_deref() == Some(edid_key)) {
        Some(i) => outputs.remove(i),
        None => OutputProfile { identity: Some(edid_key.to_string()), mode: None, active: true, touch_device: None },
    };
    outputs.insert(0, profile);
}
