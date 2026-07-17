//! Live user **preferences** — the inline-reloaded counterpart to the read-once
//! `environment.config` (settings.json). Stored in
//! `~/.config/y5.compositor/preferences.json` (same dir). Loaded fresh ([`load`])
//! rather than cached in a startup `OnceLock`: startup seeds the runtime cells,
//! the settings window re-reads on open (so terminal edits show) and writes back
//! with [`save`]; applied live, no reboot.

use serde::{Deserialize, Serialize};
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
}

impl Default for OutputProfile {
    fn default() -> Self {
        Self { identity: None, mode: None, active: true }
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
    /// Show the per-monitor FPS overlay (Settings → Performance). Off by default;
    /// each driven output gets a small top-right counter of its own draw rate.
    #[serde(default)]
    pub show_fps: bool,
    /// Release the GPU backing (dmabuf) of iced surfaces that have been hidden
    /// (off-screen or fully obstructed) for a while, re-allocating on reveal.
    /// On by default (Settings → Performance); off keeps every surface resident.
    #[serde(default = "default_release_hidden")]
    pub release_hidden_surfaces: bool,
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
    pub graphics: compositor_developer_environment_graphics_base::base::GraphicsAaConfig,
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
            show_fps: false,
            release_hidden_surfaces: true,
            outputs: Vec::new(),
            outputs_default_mode: None,
            outputs_layout: Vec::new(),
            teleport_cyclic: false,
            ime: None,
            keyboard: KeyboardLayout::default(),
            background_shader: None,
            graphics: compositor_developer_environment_graphics_base::base::GraphicsAaConfig::default(),
            protocol_foreign: default_protocol_foreign(),
            protocol_foreign_all_worlds: false,
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
    if let Some(m) = p.outputs_default_mode.as_mut() {
        if m.refresh_mhz < MIN_DEFAULT_REFRESH_MHZ {
            m.refresh_mhz = 30_000;
        }
    }
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
    p
}

/// `preferences.json`, in the same config dir as `settings.json` (honoring
/// `--config-file`/`XDG_CONFIG_HOME` via the shared resolver).
fn path() -> PathBuf {
    compositor_developer_environment_config_base::base::resolve_path()
        .with_file_name("preferences.json")
}

/// Load the preferences fresh from disk. A missing or invalid file yields the
/// defaults (so the compositor and the settings window always have sane values).
pub fn load() -> Preference {
    let prefs = std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Preference>(&raw).ok())
        .map(normalize)
        .unwrap_or_default();
    // Mirror the graphics config into the kernel-readable global.
    compositor_developer_environment_graphics_base::base::set(prefs.graphics);
    prefs
}

/// Persist `prefs` atomically (write to a sibling `.tmp`, then rename over the
/// target — a partial write can never replace a good file).
pub fn save(prefs: &Preference) -> Result<(), String> {
    // Keep the kernel-readable global in sync with every live edit.
    compositor_developer_environment_graphics_base::base::set(prefs.graphics);
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
        outputs.push(OutputProfile { identity: Some(edid_key.to_string()), mode: Some(mode), active: true });
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
        outputs.push(OutputProfile { identity: Some(edid_key.to_string()), mode: None, active });
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
        None => OutputProfile { identity: Some(edid_key.to_string()), mode: None, active: true },
    };
    outputs.insert(0, profile);
}
