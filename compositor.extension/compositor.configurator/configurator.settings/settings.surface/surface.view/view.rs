//! The settings window `IcedUi`: owns the live UI state; rendering lives in
//! `surface.chrome`. All edit messages forward to the surface handler; `Tab`, the
//! display selection, and the optimistic confirm state are handled locally.
use compositor_developer_environment_config_base::base::Environment;
use compositor_developer_environment_preference_base::base::{Ime, KeyboardLayout};
use compositor_developer_environment_keybinding_base::base::KeyRow;
use compositor_developer_environment_preference_base::base::LayoutPlacement;
use compositor_orchestration_driver_output_base::base::{ApplyResult, DisplayInfo, ModeInfo, OutputsSnapshot};
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use compositor_y5_audio_controller_interface::interface::AudioState;
use compositor_configurator_network_backend_base::base::WifiSnapshot;
use compositor_configurator_bluetooth_backend_base::base::BtSnapshot;
use compositor_configurator_hardware_gpu_base::base::{render_devices, RenderDevice};
use compositor_configurator_settings_surface_chrome::chrome;
use compositor_configurator_settings_surface_message::message::{Applied, SettingsMessage, ShaderProp, Tab};
use iced_core::{Element, Theme};

pub struct Settings {
    pub tab: Tab,
    pub cursor_sensitivity: f32,
    pub natural_scroll: bool,
    pub show_fps: bool,
    pub release_hidden: bool,
    pub env: Environment,
    /// Input-method launch command (Misc tab), persisted to preferences.json.
    pub ime: Ime,
    /// Keyboard layout (Misc tab), persisted + applied live.
    pub keyboard: KeyboardLayout,
    /// Graphics / anti-aliasing config (Graphics tab), persisted + applied live.
    pub graphics: compositor_developer_environment_graphics_base::base::GraphicsAaConfig,
    pub dirty: bool,
    /// Every connected monitor (active + connected-but-inactive), for the picker.
    pub displays: Vec<DisplayInfo>,
    /// EDID key of the monitor currently driving the compositor.
    pub active_edid: String,
    /// EDID key of the monitor selected in the picker (defaults to active).
    pub selected_display: String,
    /// Mode selected for the selected monitor (defaults to its current/first).
    pub selected_mode: Option<ModeInfo>,
    /// Whether the pending selection for the selected monitor is "Inactive"
    /// (deactivate). Mutually exclusive with a `selected_mode`. Applied by CHECK.
    pub selected_inactive: bool,
    /// The change awaiting Keep/Revert (drives the confirm bar + commit on result).
    pub pending: Option<Applied>,
    /// A STAGED activate/deactivate awaiting APPLY: `(edid_key, Some(mode)=reactivate |
    /// None=deactivate)`. Unlike `pending` (a live-provisional resolution change), this
    /// is NOT applied on CHECK — APPLY forwards the `SetActive`, REVERT discards it.
    /// Mutually exclusive with `pending`.
    pub staged_active: Option<(String, Option<ModeInfo>)>,
    pub confirming: bool,
    /// Shortcut rows for the Keys tab (id, label, default, current combo).
    pub keys: Vec<KeyRow>,
    /// Live system snapshots (pushed in per-frame via `SyncSystem`).
    pub audio: AudioState,
    pub wifi: WifiSnapshot,
    pub bt: BtSnapshot,
    /// Wi-Fi UI state: the secured network awaiting a password + the typed value.
    pub wifi_selected: Option<String>,
    pub wifi_password: String,
    /// Available render nodes with estimated GPU names (System tab picker).
    pub render_devices: Vec<RenderDevice>,
    /// Live frames-per-second (pushed by the embed), shown on the Display panel.
    pub fps: u32,
    /// Available background-shader bundle names (pushed in via `SyncShaders`).
    pub shader_options: Vec<String>,
    /// The active world's current shader override (`None` = default/built-in).
    pub shader_current: Option<String>,
    /// The selected shader's editable variables (pushed via `SyncShaderProps`).
    pub shader_props: Vec<ShaderProp>,
    /// The selected shader's WGSL source for the live preview.
    pub preview_source: String,
    /// The selected shader's compile error (active renderer), if it failed.
    pub shader_status: Option<String>,
    /// The active world's per-axis background pan inversion (pushed via
    /// `SyncWorldInvert`); mirrored on toggle so the switches reflect immediately.
    pub invert_pan_x: bool,
    pub invert_pan_y: bool,
    /// The active world's sRGB-output toggle (pushed via `SyncWorldSrgb`).
    pub srgb: bool,
    /// The cursor-teleport layout squares (Display tab canvas, multi-monitor). Each
    /// is a monitor (`identity`) placed at an abstract `(x,y)` with side `size`.
    pub layout: Vec<LayoutPlacement>,
    /// Selected placement on the canvas (highlights it + selects its monitor).
    pub selected_placement: Option<u64>,
    /// Next placement id to hand out (monotonic within the session).
    pub next_placement_id: u64,
    /// Cursor-teleport CYCLIC (wrap-around) preference — the Display-tab checkbox.
    pub cyclic: bool,
}

/// A new placement's default width/height in abstract layout units.
const PLACE_SIZE: f32 = 120.0;
/// Minimum placement extent (per axis).
const MIN_SIZE: f32 = 60.0;
/// Edge-snap radius (abstract units).
const SNAP: f32 = 12.0;

fn rects_overlap(a: &LayoutPlacement, b: &LayoutPlacement) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Snap `p`'s edges to any other placement's edges within [`SNAP`].
fn snap_edges(p: &mut LayoutPlacement, others: &[LayoutPlacement]) {
    for o in others {
        if o.id == p.id {
            continue;
        }
        for (pe, oe) in [(p.x, o.x + o.w), (p.x, o.x), (p.x + p.w, o.x), (p.x + p.w, o.x + o.w)] {
            if (pe - oe).abs() <= SNAP {
                p.x += oe - pe;
            }
        }
        for (pe, oe) in [(p.y, o.y + o.h), (p.y, o.y), (p.y + p.h, o.y), (p.y + p.h, o.y + o.h)] {
            if (pe - oe).abs() <= SNAP {
                p.y += oe - pe;
            }
        }
    }
}

/// The mode to seed the picker selection with for a display: its current mode if
/// active, else its first advertised mode.
fn default_mode(d: &DisplayInfo) -> Option<ModeInfo> {
    // Active monitor: its live mode. Otherwise the mode SAVED IN PREFERENCES for
    // this monitor, then the recommended (first advertised) as a last resort.
    d.current.or(d.preferred).or_else(|| d.available.first().copied())
}

impl Settings {
    pub fn new(env: Environment, cursor: f32, natural: bool, show_fps: bool, release_hidden: bool, snap: OutputsSnapshot, keys: Vec<KeyRow>, tab: Tab, layout: Vec<LayoutPlacement>, cyclic: bool, ime: Ime, keyboard: KeyboardLayout) -> Self {
        let active = snap.displays.iter().find(|d| d.active).cloned();
        let active_edid = active.as_ref().map(|d| d.edid_key.clone()).unwrap_or_default();
        let selected_mode = active.as_ref().and_then(default_mode);
        let next_placement_id = layout.iter().map(|p| p.id + 1).max().unwrap_or(0);
        Self {
            tab,
            cursor_sensitivity: cursor,
            natural_scroll: natural,
            show_fps,
            release_hidden,
            env,
            ime,
            keyboard,
            // Seeded from the process-global (mirrors the persisted preference).
            graphics: compositor_developer_environment_graphics_base::base::get(),
            dirty: false,
            displays: snap.displays,
            active_edid: active_edid.clone(),
            selected_display: active_edid,
            selected_mode,
            selected_inactive: false,
            pending: None,
            staged_active: None,
            confirming: false,
            keys,
            audio: AudioState::default(),
            wifi: WifiSnapshot::default(),
            bt: BtSnapshot::default(),
            wifi_selected: None,
            wifi_password: String::new(),
            render_devices: render_devices(),
            fps: 0,
            shader_options: Vec::new(),
            shader_current: None,
            shader_props: Vec::new(),
            preview_source: String::new(),
            shader_status: None,
            invert_pan_x: false,
            invert_pan_y: false,
            srgb: false,
            layout,
            selected_placement: None,
            next_placement_id,
            cyclic,
        }
    }

    /// The layout as forwarded to the handler (persist + rebuild teleport).
    fn layout_commit(&self) -> SettingsMessage {
        SettingsMessage::LayoutCommit(self.layout.clone())
    }

    fn display(&self, key: &str) -> Option<&DisplayInfo> {
        self.displays.iter().find(|d| d.edid_key == key)
    }

    /// Reset the picker selection back to the active monitor + its current mode.
    fn reset_selection(&mut self) {
        self.selected_display = self.active_edid.clone();
        self.selected_mode = self.display(&self.active_edid).and_then(default_mode);
        self.selected_inactive = false;
        self.staged_active = None;
    }

    /// Seed the pending selection for a monitor to REACTIVATE-ready: its live mode if
    /// active, else its saved / first-advertised mode. Selecting an INACTIVE monitor
    /// therefore arms CHECK to turn it back ON (`StageActive` with that mode); the
    /// "Inactive" row still stages a deactivate. (Seeding an inactive monitor to
    /// "Inactive" — the old behaviour — routed CHECK to a deactivate it can't perform,
    /// namely the last active monitor, so CHECK greyed out and the monitor looked
    /// impossible to reactivate.)
    fn seed_selection(&mut self, key: &str) {
        self.selected_mode = self.display(key).and_then(default_mode);
        self.selected_inactive = false;
    }
}

impl IcedUi for Settings {
    type Message = SettingsMessage;

    fn update(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::Tab(t) => self.tab = t,
            SettingsMessage::Fps(f) => self.fps = f,
            // Forces a re-render so the live preview animates off its wall clock.
            SettingsMessage::Tick => {}
            // Kernel outcome of the provisional apply: commit on Confirmed, reset
            // otherwise.
            SettingsMessage::ModeResult(r) => match r {
                ApplyResult::Confirmed => {
                    if let Some(p) = self.pending.take() {
                        // Update the in-memory snapshot so the changed monitor's
                        // `current` reflects the just-applied mode. Without this the
                        // snapshot is stale: re-selecting the previous mode would read
                        // as "no change", greying out CHECK, so you couldn't revert/
                        // redo in the same session.
                        for d in &mut self.displays {
                            if d.edid_key == p.edid_key {
                                d.current = Some(p.mode);
                            }
                        }
                        self.selected_mode = Some(p.mode);
                    }
                    self.confirming = false;
                }
                ApplyResult::Reverted | ApplyResult::Failed => {
                    self.pending = None;
                    self.staged_active = None;
                    self.confirming = false;
                    self.reset_selection();
                }
                ApplyResult::Provisional => {}
            },
            SettingsMessage::SetGraphics(g) => self.graphics = g,
            SettingsMessage::Cursor(v) => self.cursor_sensitivity = v,
            SettingsMessage::NaturalScroll(b) => self.natural_scroll = b,
            SettingsMessage::SetShowFps(b) => self.show_fps = b,
            SettingsMessage::SetReleaseHidden(b) => self.release_hidden = b,
            SettingsMessage::Env(e) => {
                self.env = e;
                self.dirty = true;
            }
            SettingsMessage::Ime(i) => self.ime = i,
            SettingsMessage::Keyboard(k) => self.keyboard = k,
            SettingsMessage::SelectDisplay(key) => {
                self.selected_display = key.clone();
                self.seed_selection(&key);
            }
            SettingsMessage::SelectMode(m) => {
                self.selected_mode = Some(m);
                self.selected_inactive = false;
            }
            SettingsMessage::SelectInactive => {
                self.selected_inactive = true;
                self.selected_mode = None;
            }
            SettingsMessage::Apply(a) => {
                self.pending = Some(a);
                self.staged_active = None;
                self.confirming = true;
            }
            // Activate/deactivate on CHECK: arm the confirm bar. The forwarded handler
            // applies it LIVE + arms the auto-revert watchdog (parity with a resolution
            // change); APPLY (`SetActive`) keeps it, REVERT / timeout restores it.
            // Mutually exclusive with a provisional resolution change.
            SettingsMessage::StageActive(edid, mode) => {
                self.staged_active = Some((edid, mode));
                self.pending = None;
                self.confirming = true;
            }
            SettingsMessage::Keep(_) => self.confirming = false,
            SettingsMessage::Revert => {
                self.pending = None;
                self.staged_active = None;
                self.confirming = false;
                self.reset_selection();
            }
            SettingsMessage::Rebind(id, combo) => {
                if let Some(r) = self.keys.iter_mut().find(|r| r.id == id) { r.combo = combo; }
            }
            SettingsMessage::ResetBind(id) => {
                if let Some(r) = self.keys.iter_mut().find(|r| r.id == id) { r.combo = r.default.clone(); }
            }
            SettingsMessage::SyncSystem(a, w, b) => {
                self.audio = a;
                self.wifi = w;
                self.bt = b;
            }
            // Live hotplug refresh of the monitor list. Skip while a provisional
            // change is being confirmed (the snapshot is mid-transition then), and
            // preserve the user's picker selection when that monitor still exists.
            SettingsMessage::SyncDisplays(displays) => {
                // ALWAYS refresh the raw snapshot so the picker, mode list and teleport
                // map reflect the CURRENT active set — a live-provisional activate/
                // deactivate changes the snapshot WHILE `confirming`, and the poll that
                // feeds this dedups on the snapshot, so if we dropped it here it would
                // never be re-sent after APPLY and the panel would stay stale (CHECK
                // stuck, map not updating). Only the picker's active + selection anchors
                // are held steady mid-confirm so the selection doesn't jump under the gate.
                if !self.confirming {
                    self.active_edid = displays
                        .iter()
                        .find(|d| d.active)
                        .map(|d| d.edid_key.clone())
                        .unwrap_or_default();
                    if !displays.iter().any(|d| d.edid_key == self.selected_display) {
                        self.selected_display = self.active_edid.clone();
                        self.selected_mode =
                            displays.iter().find(|d| d.edid_key == self.selected_display).and_then(default_mode);
                    }
                }
                self.displays = displays;
            }
            SettingsMessage::WifiSelect(ssid) => {
                self.wifi_selected = Some(ssid);
                self.wifi_password.clear();
            }
            SettingsMessage::SyncShaders(options, current) => {
                self.shader_options = options;
                self.shader_current = current;
            }
            // UI mirror; also forwarded to persist + rebuild. Empty = default.
            SettingsMessage::SetWorldShader(s) => {
                self.shader_current = if s.is_empty() { None } else { Some(s) };
            }
            SettingsMessage::SyncShaderProps(props) => self.shader_props = props,
            SettingsMessage::SyncShaderPreview(src) => self.preview_source = src,
            SettingsMessage::SyncShaderStatus(status) => self.shader_status = status,
            // Pan-inversion: pushed state, plus the UI mirror of each toggle (both
            // also forwarded to persist + flip the live background).
            SettingsMessage::SyncWorldInvert(x, y) => { self.invert_pan_x = x; self.invert_pan_y = y; }
            SettingsMessage::SetWorldInvertPanX(v) => self.invert_pan_x = v,
            SettingsMessage::SetWorldInvertPanY(v) => self.invert_pan_y = v,
            SettingsMessage::SyncWorldSrgb(v) => self.srgb = v,
            SettingsMessage::SetWorldSrgb(v) => self.srgb = v,
            // UI mirror of an edit; also forwarded to persist + drive the shader.
            SettingsMessage::SetWorldShaderParams(values) => {
                for p in &mut self.shader_props {
                    if let Some((_, v)) = values.iter().find(|(n, _)| n == &p.name) {
                        p.value = *v;
                    }
                }
            }
            SettingsMessage::WifiPassword(p) => self.wifi_password = p,
            SettingsMessage::WifiConnect(..) => {
                self.wifi_selected = None;
                self.wifi_password.clear();
            }
            // --- Teleport-layout canvas edits (UI-local; APPLY LAYOUT forwards) ---
            SettingsMessage::LayoutPlace(edid, x, y) => {
                let id = self.next_placement_id;
                self.next_placement_id += 1;
                let mut p = LayoutPlacement { id, identity: edid, x, y, w: PLACE_SIZE, h: PLACE_SIZE };
                // Nudge right until it doesn't overlap an existing placement.
                while self.layout.iter().any(|o| rects_overlap(&p, o)) {
                    p.x += PLACE_SIZE + SNAP;
                }
                self.selected_display = p.identity.clone();
                self.selected_mode = self.display(&self.selected_display).and_then(default_mode);
                self.selected_placement = Some(id);
                self.layout.push(p);
            }
            SettingsMessage::LayoutMove(id, x, y) => {
                if let Some(mut moved) = self.layout.iter().find(|p| p.id == id).cloned() {
                    moved.x = x;
                    moved.y = y;
                    let others: Vec<_> = self.layout.iter().filter(|p| p.id != id).cloned().collect();
                    snap_edges(&mut moved, &others);
                    // Reject the move if it would overlap another square.
                    if !others.iter().any(|o| rects_overlap(&moved, o)) {
                        if let Some(slot) = self.layout.iter_mut().find(|p| p.id == id) {
                            slot.x = moved.x;
                            slot.y = moved.y;
                        }
                    }
                }
            }
            SettingsMessage::LayoutResize(id, w, h) => {
                let (w, h) = (w.max(MIN_SIZE), h.max(MIN_SIZE));
                if let Some(mut resized) = self.layout.iter().find(|p| p.id == id).cloned() {
                    resized.w = w;
                    resized.h = h;
                    let others: Vec<_> = self.layout.iter().filter(|p| p.id != id).cloned().collect();
                    if !others.iter().any(|o| rects_overlap(&resized, o)) {
                        if let Some(slot) = self.layout.iter_mut().find(|p| p.id == id) {
                            slot.w = w;
                            slot.h = h;
                        }
                    }
                }
            }
            SettingsMessage::LayoutSelect(id) => {
                self.selected_placement = Some(id);
                if let Some(p) = self.layout.iter().find(|p| p.id == id) {
                    self.selected_display = p.identity.clone();
                    self.selected_mode = self.display(&self.selected_display).and_then(default_mode);
                }
            }
            SettingsMessage::LayoutRemove(id) => {
                self.layout.retain(|p| p.id != id);
                if self.selected_placement == Some(id) {
                    self.selected_placement = None;
                }
            }
            // Forwarded (persist + rebuild teleport): the UI already holds the data.
            SettingsMessage::LayoutCommit(_) => {}
            // Optimistic: reflect the cyclic checkbox immediately (also forwarded).
            SettingsMessage::SetCyclic(b) => self.cyclic = b,
            // APPLY of a staged activate/deactivate (also forwarded → kernel reconciles):
            // drop the confirm bar + clear the stage. The kernel applies it and the next
            // `SyncDisplays` refresh reflects the new active set.
            SettingsMessage::SetActive(..) => {
                self.staged_active = None;
                self.confirming = false;
            }
            // Forwarded-only actions: no local UI state change.
            SettingsMessage::SetDefaultSink(_)
            | SettingsMessage::SetSinkVolume(_, _)
            | SettingsMessage::SetSinkMute(_, _)
            | SettingsMessage::WifiEnable(_)
            | SettingsMessage::WifiScan
            | SettingsMessage::BtPower(_)
            | SettingsMessage::BtScan(_)
            | SettingsMessage::BtPair(_)
            | SettingsMessage::BtConnect(_)
            | SettingsMessage::Close => {}
        }
    }

    fn view(&self) -> Element<'_, SettingsMessage, Theme, Renderer> {
        chrome::render(
            self.tab,
            self.dirty,
            self.cursor_sensitivity,
            self.natural_scroll,
            self.show_fps,
            self.release_hidden,
            &self.env,
            &self.displays,
            &self.active_edid,
            &self.selected_display,
            self.selected_mode,
            self.pending.as_ref(),
            self.staged_active.as_ref(),
            self.confirming,
            &self.keys,
            &self.audio,
            &self.wifi,
            &self.bt,
            self.wifi_selected.as_deref(),
            &self.wifi_password,
            &self.render_devices,
            self.fps,
            &self.layout,
            self.selected_placement,
            self.cyclic,
            self.selected_inactive,
            &self.ime,
            &self.keyboard,
            &self.shader_options,
            self.shader_current.as_deref(),
            &self.shader_props,
            &self.preview_source,
            self.shader_status.as_deref(),
            self.invert_pan_x,
            self.invert_pan_y,
            self.srgb,
            &self.graphics,
        )
    }
}
