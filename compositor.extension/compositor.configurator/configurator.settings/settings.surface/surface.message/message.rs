//! The settings-window message type, shared by the view + tab builders + the
//! surface protocol/handler. iced-free so the protocol crate can name it.
use compositor_model_environment_config_base::base::Environment;
use compositor_model_environment_preference_base::base::{Ime, KeyboardLayout, PenBindTarget, PenConfig};
use compositor_orchestration_driver_output_base::base::{ApplyResult, DisplayInfo, ModeInfo, TouchDeviceInfo};

/// A provisional per-monitor mode change the user can Keep/Revert: the target
/// monitor (by EDID identity key) and the mode to drive it at. Multi-output: every
/// output is independently driven, so this is always an in-place mode change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub edid_key: String,
    pub mode: ModeInfo,
}
use compositor_y5_audio_controller_interface::interface::AudioState;
use compositor_configurator_network_backend_base::base::WifiSnapshot;
use compositor_configurator_bluetooth_backend_base::base::BtSnapshot;

/// Sub-sections of the INPUT module, shown as a tab bar inside the Input panel.
/// Carried INSIDE `Tab::Input` (not a separate field) so the selected sub-tab
/// round-trips through the session `SettingsState` u8 — restored on the next
/// settings open exactly like the top-level module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputTab {
    /// Pointer speed + touchpad natural scroll.
    #[default]
    Mouse,
    /// Touchscreen: pan speed, linear pan, and the touch↔display link.
    Touch,
    /// Keyboard shortcut bindings.
    Keyboard,
    /// Pen / tablet overrides (stylus buttons, dial, pressure threshold).
    Pen,
}

/// Sub-sections of the GRAPHICS module. Carried inside `Tab::Graphics` for the
/// same reason as [`InputTab`] — the selection round-trips through the session
/// `SettingsState` u8 and is restored on the next open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GraphicsTab {
    /// Anti-aliasing: the minification method and its per-zoom knobs.
    #[default]
    Aa,
    /// FSR: the EASU / RCAS magnification filters.
    Fsr,
    /// Page-flip policy — tearing, its pacing fallback, and target tagging.
    Pacing,
}

/// The settings modules shown in the sidebar (design: SYSTEM CONFIGURATION).
/// `Input` merges the former Cursor + Keys (now sub-tabbed via [`InputTab`]);
/// `System` is the Environment editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Display,
    Audio,
    Input(InputTab),
    Network,
    Bluetooth,
    Performance,
    System,
    Misc,
    /// Keyboard layouts (xkb) + the launched input method.
    Language,
    /// Per-world settings for the active world (background shader, …).
    World,
    /// Graphics tuning for the pannable world: AA, FSR, and the flip policy.
    Graphics(GraphicsTab),
}

impl Tab {
    /// Stable index for session persistence in the driver `SettingsState`
    /// (orchestration can't name `Tab`, so the selected module round-trips as a `u8`).
    pub fn to_index(self) -> u8 {
        match self {
            Tab::Display => 0, Tab::Audio => 1, Tab::Input(InputTab::Mouse) => 2, Tab::Network => 3,
            Tab::Bluetooth => 4, Tab::Performance => 5, Tab::System => 6, Tab::Misc => 7,
            Tab::World => 8, Tab::Graphics(GraphicsTab::Aa) => 9, Tab::Language => 10,
            // Extend past the original range so existing persisted indices (Mouse = 2,
            // Graphics = 9) stay stable; only new sub-tabs claim fresh slots.
            Tab::Input(InputTab::Touch) => 11, Tab::Input(InputTab::Keyboard) => 12,
            Tab::Input(InputTab::Pen) => 13,
            Tab::Graphics(GraphicsTab::Fsr) => 14, Tab::Graphics(GraphicsTab::Pacing) => 15,
        }
    }
    pub fn from_index(i: u8) -> Self {
        match i {
            1 => Tab::Audio, 2 => Tab::Input(InputTab::Mouse), 3 => Tab::Network, 4 => Tab::Bluetooth,
            5 => Tab::Performance, 6 => Tab::System, 7 => Tab::Misc, 8 => Tab::World,
            9 => Tab::Graphics(GraphicsTab::Aa), 10 => Tab::Language,
            11 => Tab::Input(InputTab::Touch), 12 => Tab::Input(InputTab::Keyboard),
            13 => Tab::Input(InputTab::Pen),
            14 => Tab::Graphics(GraphicsTab::Fsr), 15 => Tab::Graphics(GraphicsTab::Pacing),
            _ => Tab::Display,
        }
    }
}

/// How a shader `@prop` is edited in the Current-World panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderPropKind {
    Float,
    Bool,
}

/// One editable shader variable, flattened for the iced UI: the control kind,
/// which param slot it drives, its range, and the current value.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderProp {
    /// The raw `@prop` name — the persistence key for this variable's value.
    pub name: String,
    pub label: String,
    pub kind: ShaderPropKind,
    pub slot: usize,
    pub min: f32,
    pub max: f32,
    pub value: f32,
}

#[derive(Clone, Debug)]
pub enum SettingsMessage {
    /// Switch the visible tab (UI-only, not forwarded).
    Tab(Tab),
    /// Live frames-per-second, pushed by the embed (UI-only, not forwarded).
    Fps(u32),
    /// Per-frame redraw tick pushed by the embed while the Current-World tab is
    /// open (UI-only, not forwarded): animates the live preview.
    Tick,
    /// Kernel result of the last mode Apply, pushed by the embed (UI-only): drops
    /// the confirm bar + restores the shown mode when a mode wasn't kept.
    ModeResult(ApplyResult),
    /// Live cursor speed multiplier (forwarded: applied + persisted at once).
    Cursor(f32),
    /// Live touchpad natural-scroll (forwarded).
    NaturalScroll(bool),
    /// Live touch pan-speed multiplier (forwarded: persisted to preferences.json,
    /// read live per touch event).
    TouchPanSpeed(f32),
    /// Toggle linear (strict, no-coast) touch pan (forwarded; persisted).
    TouchLinearPan(bool),
    /// On-screen keyboard size multiplier (forwarded; persisted, read live).
    OskSize(f32),
    /// Auto-summoned OSK floats in world position near the caret (forwarded; persisted).
    OskWorldPosition(bool),
    /// Toggle the per-monitor FPS overlay (forwarded; persisted to preferences).
    SetShowFps(bool),
    /// Toggle releasing hidden iced surfaces' GPU memory (forwarded; persisted).
    SetReleaseHidden(bool),
    /// Fractional-scale strategy for invisible windows: "off" | "optimized" | "full".
    SetFractionalInvisible(String),
    /// Background triple buffering (forwarded; persisted). Carries the whole
    /// struct like `SetFlip`, so the enable, the presets and the individual knobs
    /// share one variant. Knobs apply live; `enabled` takes effect on RESTART.
    SetTripleBuffer(compositor_model_environment_background_base::base::TripleBufferBackground),
    /// UI (iced + bevy) triple buffering. Separate from `SetTripleBuffer`: they
    /// are different subsystems with different knobs, and only the name is shared.
    SetInterfaceBuffer(compositor_model_environment_interface_base::base::TripleBufferUI),
    /// Page-flip policy — tearing plus its pacing fallback (forwarded; persisted
    /// to preferences.json and mirrored live into the scanout global, no reboot).
    /// Carries the whole struct like `Env`/`Ime`, so all eight fields across both
    /// sections share one variant instead of eight.
    SetFlip(compositor_model_environment_tearing_config::config::Config),
    /// A full edited Environment to write back to settings.json (forwarded;
    /// sets the reboot-dirty banner). Carrying the whole struct keeps one
    /// message variant instead of 19 field-specific ones.
    Env(Environment),
    /// A full edited input-method command (Misc tab) to persist to preferences.json
    /// (forwarded). Carries the whole `Ime` like `Env`, so exec + args edits share one
    /// variant. Applied on the next compositor start.
    Ime(Ime),
    /// A full edited keyboard-layout preference (Misc tab) to persist to
    /// preferences.json AND apply live (forwarded). Carries the whole
    /// `KeyboardLayout` like `Ime`/`Env`, so the source toggle + layout/variant/
    /// options edits share one variant.
    Keyboard(KeyboardLayout),
    /// Select a monitor in the Display picker (UI-local: syncs the mode list).
    SelectDisplay(String),
    /// Select a mode for the selected monitor (UI-local).
    SelectMode(ModeInfo),
    /// Provisionally apply the selected monitor + mode (forwarded): a mode change
    /// on the active monitor, or an active-output switch to another monitor.
    Apply(Applied),
    /// Keep the provisional change (forwarded: confirm + persist preferred
    /// monitor and/or per-EDID mode).
    Keep(Applied),
    /// Revert the provisional change (forwarded: reverts whichever gate is armed).
    Revert,
    /// Rebind a shortcut: `(action_id, combo_string)` (forwarded: parsed +
    /// persisted to keybinding.json).
    Rebind(String, String),
    /// Reset a shortcut to its default (forwarded: clears the override).
    ResetBind(String),
    /// Live system snapshots pushed in by the per-frame reconciler (NOT forwarded).
    SyncSystem(AudioState, WifiSnapshot, BtSnapshot),
    /// Live connected-monitor list pushed in on hotplug (NOT forwarded): refreshes
    /// the Display picker for the open session.
    SyncDisplays(Vec<DisplayInfo>),
    /// Live connected touch-device list pushed in on hotplug (NOT forwarded):
    /// refreshes the Display tab's per-monitor touch-claim control.
    SyncTouchDevices(Vec<TouchDeviceInfo>),
    /// Claim (`Some`) or release (`None`) a touch device for a monitor (forwarded):
    /// `(edid_key, device_id)`. Persists to `preferences.json` and re-routes touch.
    ClaimTouch(String, Option<String>),
    /// Available background-shader bundles + the active world's current selection,
    /// pushed in by the embed (NOT forwarded): populates the shader picker.
    SyncShaders(Vec<String>, Option<String>),
    /// Set the CURRENT world's background shader (forwarded: writes the per-world
    /// record + rebuilds the background). Empty string = default/built-in.
    SetWorldShader(String),
    /// The current shader's editable variables, pushed by the embed (NOT
    /// forwarded): renders the Current-World variable controls.
    SyncShaderProps(Vec<ShaderProp>),
    /// The selected shader's WGSL source for the live preview, pushed by the
    /// embed (NOT forwarded). Empty until the first sync.
    SyncShaderPreview(String),
    /// The selected shader's compile status, pushed by the embed (NOT forwarded):
    /// `Some(error)` when it failed for the active renderer (built-in is running).
    SyncShaderStatus(Option<String>),
    /// Set the current world's shader variables, keyed by `@prop` name (forwarded:
    /// persists + drives the live background, no rebuild).
    SetWorldShaderParams(Vec<(String, f32)>),
    /// Invert the current world's background pan on the horizontal / vertical axis
    /// (forwarded: persists + flips the live background's pan on that axis, no rebuild).
    SetWorldInvertPanX(bool),
    SetWorldInvertPanY(bool),
    /// Gamma-encode the current world's background to sRGB (forwarded: persists +
    /// flips the live background, no rebuild). On = brighter, preview-matching output.
    SetWorldSrgb(bool),
    /// Render the current world's background with the built-in parallax's cheap
    /// variant (forwarded: persists + swaps the live background's SPIR-V, no
    /// rebuild). Only the built-in has one, so the toggle is disabled when the
    /// world has a shader selected.
    SetWorldOptimized(bool),
    /// The current world's pan-inversion state (invert X, invert Y), pushed by the
    /// embed (NOT forwarded): sets the toggles when the panel opens / the world switches.
    SyncWorldInvert(bool, bool),
    /// The current world's sRGB-output state, pushed by the embed (NOT forwarded).
    SyncWorldSrgb(bool),
    /// The current world's optimized-variant state, pushed by the embed (NOT
    /// forwarded): `(on, available)`. `available` is false when the selected shader
    /// declares no `@optimized` knobs, which greys the toggle out.
    SyncWorldOptimized(bool, bool),
    /// Audio (forwarded): make a sink default / set a sink's volume / mute a sink.
    SetDefaultSink(String),
    SetSinkVolume(String, f32),
    SetSinkMute(String, bool),
    /// Wi-Fi: enable/scan/connect are forwarded; Select/Password are UI-local.
    WifiEnable(bool),
    WifiScan,
    WifiSelect(String),
    WifiPassword(String),
    WifiConnect(String, String),
    /// Bluetooth (forwarded): power, scan, pair/connect by device path.
    BtPower(bool),
    BtScan(bool),
    BtPair(String),
    BtConnect(String),
    /// Close the settings window (forwarded).
    Close,

    // --- Cursor-teleport layout canvas (Display tab, multi-monitor) ------------
    /// Drop monitor `edid_key` onto the canvas at abstract-layout `(x, y)` as a new
    /// square (UI-local: appends a placement, snapped + nudged off overlaps).
    LayoutPlace(String, f32, f32),
    /// Move placement `id` to abstract-layout `(x, y)` (UI-local: snap + no-overlap).
    LayoutMove(u64, f32, f32),
    /// Resize placement `id` to width `w` × height `h` (UI-local: min size + no-overlap).
    /// A free rectangle — teleport geometry only, never scale/resolution.
    LayoutResize(u64, f32, f32),
    /// Select placement `id` (UI-local: also selects its monitor so the mode/res
    /// controls below populate).
    LayoutSelect(u64),
    /// Remove placement `id` from the canvas (UI-local).
    LayoutRemove(u64),
    /// Commit the whole arrangement (forwarded on drag-end): persisted to
    /// `preferences.json` and rebuilt into the live teleport layout.
    LayoutCommit(Vec<compositor_model_environment_preference_base::base::LayoutPlacement>),
    /// UI-LOCAL: select the "Inactive" row for the selected monitor (a pending
    /// deactivate that CHECK CHANGES then applies), like `SelectMode` for a mode.
    SelectInactive,
    /// Forwarded (on CHECK CHANGES): PROVISIONALLY apply an active-state change for the
    /// selected monitor and arm the confirm bar. `None` = deactivate; `Some(mode)` =
    /// (re)activate at that mode. The handler applies it LIVE (persist + reconcile) and
    /// arms the auto-revert watchdog, so activate/deactivate has the SAME live-provisional
    /// CHECK → APPLY/REVERT gate as a resolution change. APPLY forwards `SetActive` (keep);
    /// REVERT / timeout restores the prior state. Mutually exclusive with `pending`.
    StageActive(String, Option<ModeInfo>),
    /// Forwarded (on APPLY): KEEP the provisional activate/deactivate — disarms the
    /// auto-revert watchdog (the change was already applied on CHECK). The payload is the
    /// staged `(edid, mode)` for the UI; the handler needs only to disarm.
    SetActive(String, Option<ModeInfo>),
    /// Forwarded: toggle the cursor-teleport CYCLIC (wrap-around) preference.
    SetCyclic(bool),
    /// A full edited graphics/anti-aliasing config (Graphics tab) to persist to
    /// preferences.json AND apply live (forwarded). Carries the whole struct so
    /// the method dropdown + every knob share one variant.
    SetGraphics(compositor_model_environment_graphics_base::base::GraphicsAaConfig),
    /// Forwarded (Misc tab): the `protocol_foreign` preference — `"enabled"` or
    /// `"disabled"` — gating the wlr + ext foreign-toplevel (dock) protocols.
    /// Persisted to preferences.json; takes effect on the next start.
    SetProtocolForeign(String),
    /// Forwarded (Misc tab): `protocol_foreign_all_worlds` — when on, the foreign-toplevel
    /// advertisement lists windows from ALL worlds, not just the active one. Persisted;
    /// takes effect on the next start.
    SetProtocolForeignAllWorlds(bool),
    /// UI-LOCAL (Language tab): open/close the "add a language" layout picker.
    LangPickerOpen(bool),
    /// UI-LOCAL (Language tab): the layout-picker search query.
    LangSearch(String),
    /// A full edited pen/tablet config (Pen tab) to persist to preferences.json AND
    /// apply live (forwarded). Carries the whole `PenConfig` so every control shares
    /// one variant.
    SetPen(PenConfig),
    /// Pen tab: start capturing the next keyboard combo to bind to `target`
    /// (forwarded: arms the compositor's capture, which applies the result + syncs).
    PenCaptureKey(PenBindTarget),
    /// Pen tab: start capturing the next pad button press (forwarded).
    PenCapturePad,
    /// Pen tab: cancel an in-progress capture (forwarded).
    PenCaptureCancel,
    /// The live pen config, pushed by the reconciler (UI-local) after a capture applies
    /// it compositor-side, so the tab reflects the new binding.
    SyncPen(PenConfig),
}
