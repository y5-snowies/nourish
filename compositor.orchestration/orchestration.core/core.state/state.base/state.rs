use compositor_y5_audio_controller_interface::interface::AudioController;
use compositor_y5_audio_controller_interface::media::MediaController;
use compositor_orchestration_environment_type_base::base::Environment;
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::multigpu::GpuManager;
use smithay::backend::renderer::multigpu::gbm::GbmGlesBackend;
use smithay::desktop::{Window, layer_map_for_output};
use compositor_y5_graphic_capture_registry::CaptureRegistry;
use compositor_y5_lock_state_base::state::LockState;

use crate::Loop;
use smithay::reexports::calloop::{EventLoop, LoopSignal, RegistrationToken};
use smithay::reexports::wayland_server::DisplayHandle;
use std::cell::RefCell;
use std::ffi::OsString;
use std::rc::Rc;
use std::time::Instant;
use compositor_introspection_sampler_window_base::sampler::SampleResult;
use compositor_y5_camera_state_base::state::Camera;
use compositor_y5_canvas_state_base::state::CanvasState;
use compositor_orchestration_seat_pointer_snapshot::snapshot::CursorSnapshot;
use compositor_orchestration_seat_gesture_scroll::scroll::FingerScrollRamp;
use compositor_support_smithay_dispatch_wire_base::wire::Wire;
use compositor_support_smithay_dispatch_wire_trait::wire_trait::WireTrait;

pub struct Loader {
    pub socket_name: OsString,
    // pub socket_name_proprietary: OsString,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,
}

/// Deferred world-selection-screen request, drained in the GLES prepare phase
/// (where a renderer + the capture registry are available).
/// Opening is deferred so the current world's framebuffer can be snapshotted for
/// its thumbnail before we switch away from it.
#[derive(Clone, Copy)]
pub enum SetPickerRequest {
    Open,
}
/// The loop object must implement various things like decoration, "resize and movement" modes, and input management.
/// The trait must be implemented in this crate and can be delegated if necessary.
/// While the per-region render loop draws one viewport pane (split / floating),
/// this overrides the focus accessors: `camera()`/`camera_mut()`/`size_context()`
/// resolve to THIS pane's slot + region rect instead of the focused slot + full
/// output. `None` everywhere outside that loop (normal full-output rendering and
/// all input/logic paths). This is the single seam that lets one world render
/// through several cameras into several screen regions in one frame.
#[derive(Clone, Copy)]
pub struct RenderTarget {
    pub slot: compositor_y5_viewport_state_base::state::SlotId,
    /// Region top-left in logical screen pixels.
    pub origin_logical: (f64, f64),
    /// Region size in physical pixels.
    pub size_physical: (f64, f64),
}

/// The stable per-monitor identity of a smithay `Output`: its EDID key
/// "make model serial", matching `DisplayInfo::edid_key` / `MonitorIdentity::key()`
/// and the per-monitor preference keys. The kernel's EDID identity falls back to the
/// connector name for the serial when the EDID is unreadable / serial-less, so this
/// is UNIQUE per physical output even for two identical / EDID-less monitors — the
/// key the per-output render loop, coordinate contexts, settings and teleport all
/// resolve against.
pub fn output_key(output: &smithay::output::Output) -> compositor_orchestration_driver_output_base::base::OutputKey {
    let p = output.physical_properties();
    format!("{} {} {}", p.make, p.model, p.serial_number)
}

/// How long a window must go unseen on every monitor before it is told
/// `xdg_toplevel.suspended` (see [`Orchestrator::refresh_suspended`]).
///
/// Deliberately long. What is bought is a client that has stopped repainting, which is
/// worth having within seconds rather than within frames, and every second shaved off
/// buys a little less power for a lot more risk of telling a window it is invisible
/// moments before the user pans back to it. Cleared on the first sighting, so this is a
/// floor on how long absence must last, never on how long a reveal takes.
const SUSPEND_DWELL: f32 = 5.0;

/// The overlap rect handed to `SpaceElement::output_enter` for a window that IS on an
/// output — deliberately unbounded (see [`Orchestrator::refresh_space`]).
///
/// smithay does not treat that rect as a one-off: it stores it and, on every later
/// `refresh_elements`, re-tests each individual surface of the window's tree against it
/// — and each popup against it SHIFTED by the popup's offset — sending
/// `wl_surface.leave` to whatever falls outside. Membership has already been decided by
/// then, so the rect must only say "all of it", and no rect derived from world
/// coordinates can promise that for a tree whose subsurfaces and popups reach
/// arbitrarily far. Kept well inside `i32` so the saturating arithmetic in
/// `Rectangle::overlaps` never clamps it back.
fn unclipped() -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
    smithay::utils::Rectangle::new(
        smithay::utils::Point::from((i32::MIN / 4, i32::MIN / 4)),
        smithay::utils::Size::from((i32::MAX / 2, i32::MAX / 2)),
    )
}

pub struct Orchestrator {
    pub start_time: std::time::Instant,
    /// When each window was last ON-PANE-GRACE, by uuid — the dwell behind
    /// `xdg_toplevel.suspended` (see [`Self::refresh_suspended`]). Session-wide rather
    /// than per world, because a window keeps its uuid across a world move and the
    /// state it carries should not reset when it does.
    pub last_on_pane_awake: std::collections::HashMap<uuid::Uuid, f32>,
    /// Windows currently TOLD they are `xdg_toplevel.suspended`, by uuid — what makes
    /// that sweep edge-triggered rather than a per-frame re-assertion. Kept here rather
    /// than read back off the toplevel because the honest question is "what have we
    /// published", and the surface's own state answers a different one: it lags by a
    /// configure round-trip, so a window would be re-set on every frame until it acked.
    pub suspended: std::collections::HashSet<uuid::Uuid>,
    /// Active per-region render override (see [`RenderTarget`]). Set only inside
    /// the `scene.frame` region loop.
    pub render_target: Option<RenderTarget>,
    /// The physical output currently being drawn, by [`output_key`]. Set by the
    /// kernel's per-output render loop around each output's `scene()` call and
    /// cleared after (mirrors [`RenderTarget`], but at output granularity).
    /// [`current_output`](Self::current_output) resolves THIS output's mode
    /// size/scale while set, so the coordinate contexts build against the framebuffer
    /// being drawn. `None` outside the render loop and on single-output hardware,
    /// where the resolver falls back to the sole output. The shared `Viewports`
    /// view state is unchanged — this only selects which output's geometry is used.
    pub render_output: Option<compositor_orchestration_driver_output_base::base::OutputKey>,
    /// Per-output monotonic scene-build counter — see [`Orchestrator::next_frame_serial`].
    pub frame_serial: std::collections::HashMap<compositor_orchestration_driver_output_base::base::OutputKey, u64>,
    /// The physical output currently under the cursor, by [`output_key`]. Updated by
    /// the pointer path as the cursor crosses between monitors (teleport). Selects
    /// which output's size/scale the input-path contexts use. `None` until the first
    /// crossing resolves it; the resolver falls back to the sole/primary output.
    pub cursor_output: Option<compositor_orchestration_driver_output_base::base::OutputKey>,
    /// Saved pointer-cursor state while a TOUCH sequence owns the single active
    /// output. `Some` ⇒ the cursor is "hidden" and touch drives the touched panel;
    /// the snapshot is restored (position + output) the moment a real pointer/mouse
    /// event arrives, so the cursor reappears exactly where it was. The behaviour
    /// lives in `seat.pointer/pointer.restore` (`TouchCursor`); only the value is here.
    pub saved_cursor: Option<CursorSnapshot>,
    /// Softens the start of a two-finger touchpad scroll forwarded to a window
    /// (libinput dumps the accumulated pre-recognition distance in the first
    /// event, so the gesture lurches at the start). Policy lives in
    /// `seat.gesture/gesture.scroll`.
    pub finger_scroll_ramp: FingerScrollRamp,
    pub status: Status,
    /// One-shot request to run the renderer-free lock engage (`lock_logical`) off
    /// the render loop. The lock keybinding sets `Status::Locked` synchronously and
    /// flips this; `wire.input` drains it and schedules the engage on an idle (the
    /// keyboard crates can't call `lock.interface` — it depends back on them).
    pub lock_engage: bool,
    /// Wakes the native control-plane ping source that drains the display
    /// request queues (output mode / preferred-monitor switch / lid apply) OFF
    /// the render and input paths. Set once by the native backend; stays `None`
    /// on winit (no DRM modeset there). Producers call `ping_control()` right
    /// after queuing a request instead of relying on the next input event to
    /// drain it.
    pub control_ping: Option<smithay::reexports::calloop::ping::Ping>,
    // Deferred request to open the world-selection screen on a coming draw.
    pub __set_picker: Option<SetPickerRequest>,
    pub status_session: StatusSession,
    /// Per-seat touchpad swipe accumulator (libinput Begin→Update*→End spans
    /// multiple input dispatches). World-agnostic raw delta; the y5 gesture
    /// handler turns it into a directional-view action at end-of-swipe.
    pub gesture: compositor_orchestration_seat_gesture_state::state::GestureAccumulator,
    /// Multi-finger touchscreen session: active contacts + the role/mode the
    /// current touch sequence committed to (client-forward vs pointer-emu vs
    /// canvas gesture). Feeds the same trackpad gesture handlers as `gesture`.
    pub touch: compositor_orchestration_seat_gesture_touch::touch::TouchTracker,
    pub loader: Loader,
    /// The world set (phase 3, document/ARCHITECTURE.md). The active world
    /// hosts the kernel systems; grows per-output/lock/selection worlds later.
    pub worlds: compositor_orchestration_world_manager_base::manager::WorldManager,
    /// Per-world keyboard-focus memory: the window that held keyboard focus when
    /// each world was last left. Keyboard focus is a single global on the seat, so a
    /// world switch otherwise strands focus on the outgoing world's window. The
    /// `WORLD_SWITCHED` rim handler saves the outgoing world's focus here and restores
    /// the incoming world's (see `Wire::apply_world_switch_focus`). Entries for
    /// destroyed windows are pruned on restore (stale `Window` handles read `!alive`).
    pub world_focus_memory: std::collections::HashMap<uuid::Uuid, Window>,
    /// KernelData: smithay wiring handles behind storage tokens (read-only for
    /// systems; populated post-init by the loader via smithay.data populate()).
    pub kernel: compositor_support_system_storage_slot_base::base::Storage,
    /// TRANSITIONAL legacy bus: deferred messages whose receivers still need
    /// the whole Loop. Dies with this struct (phase 6).
    pub bus: compositor_orchestration_bus_legacy_base::legacy::LegacyBus<crate::Loop>,
    pub pilot_tick: u64,
    // rpc is now driver data in `kernel` (compositor_orchestration_driver_remote_base).
    // sampler is now driver data in `kernel` (compositor_orchestration_driver_introspection_base).
    pub storage: compositor_orchestration_storage_state_base::state::Storage,

    // __gpu_ref is now driver data in `kernel` (GPU_BINDING token, above).
    // capture_registry/capture are now driver data in `kernel`
    // (compositor_orchestration_driver_capture_base CAPTURE_REGISTRY / CAPTURE).

    // audio/media are now driver data in `kernel` (compositor_orchestration_driver_audio_base).
    pub environment: Environment,
    /// Live user preferences (cursor speed, touchpad natural-scroll, per-EDID
    /// output modes) — the inline-reloaded counterpart to the read-once
    /// `environment`. Seeded at startup from `preference::load()`, refreshed from
    /// disk whenever the settings window opens, and written live by the settings
    /// handler (which then persists it). Read per-event by motion.rs / axis.rs;
    /// reachable from both the input path and the UI handler via `&mut Loop`.
    pub preference: compositor_model_environment_preference_base::base::Preference,
    /// Live keyboard-shortcut overrides (keybinding.json). Seeded at startup,
    /// refreshed whenever the settings window opens, written by the settings
    /// handler. Read by the overlay shortcut path on every keypress (parse-or-
    /// default). The inline-reloaded counterpart to the read-once settings.
    pub keybinding: compositor_model_environment_keybinding_base::base::KeyBindings,
    // Cursor-teleport state moved OUT of the Orchestrator into `driver.output` storage
    // tokens: the layout + current placement (`TELEPORT_LAYOUT` / `CURSOR_PLACEMENT`, in
    // kernel storage) and the suppression lock (`TELEPORT_SUPPRESS`, a refcount in world
    // storage that any system raises to pin the cursor — see `teleport_suppressed`).
    // They are output-arrangement / seat state, not core state.
}

pub struct StateDRMBinding {
    pub gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    pub primary: DrmNode,
}

/// GPU driver data: the DRM multi-GPU binding (for dmabuf import) lives in the
/// kernel/driver storage by token, not as an Orchestrator field. `Option` —
/// populated post-init by the active backend (winit/udev). The token type lives
/// here with `StateDRMBinding` to avoid a dependency cycle.
pub static GPU_BINDING: compositor_support_system_storage_token_base::base::Token<Option<Rc<RefCell<StateDRMBinding>>>> =
    compositor_support_system_storage_token_base::base::Token::new();
pub static GPU_BINDING_MUT: compositor_support_system_storage_token_base::base::TokenMut<Option<Rc<RefCell<StateDRMBinding>>>> =
    compositor_support_system_storage_token_base::base::TokenMut::new(&GPU_BINDING);

pub enum Status {
    Running,

    Locked {
        pending: bool,
        sleep: bool,
        time: Instant,
    },
    Unlock {
        // pending: bool,
        time: Instant,
    },
    Sleep {
        // Always locks on sleep.
        pending: bool,
    },
    Terminate,
}
pub enum StatusSession {
    Active,
    Paused,
}

/// Announced when the spawn-target world changes (i.e. the window Space the foreign
/// mirror advertises — see `Orchestrator::space_state`). The rim's foreign reconciler
/// listens for this (registered in the loader) instead of polling a change token.
pub struct WorldSwitched;
compositor_support_system_channel_token_base::y5_channel!(pub WORLD_SWITCHED, WORLD_SWITCHED_TX: WorldSwitched);

impl Orchestrator {
    pub fn new(
        environment: Environment,
        nested: bool,
        loader: Loader,

        rpc_broadcast: tokio::sync::broadcast::Sender<
            compositor_remote_message_server_base::message::Message,
        >,
        mut kernel_data: compositor_support_system_storage_slot_base::base::Storage,
        worlds: compositor_orchestration_world_manager_base::manager::WorldManager,
    ) -> Self {
        let start_time = std::time::Instant::now();

        // Live user preferences (cursor speed, touch natural-scroll) loaded fresh
        // from preferences.json to seed the runtime cells below.
        let prefs = compositor_model_environment_preference_base::base::load();
        // Seed the process-global default background shader so `background.two`'s
        // system (no preference access in `update()`) can resolve it per world.
        compositor_model_stats_registry_base::base::set_background_shader_default(
            prefs.background_shader.clone(),
        );
        // Same seam: the background triple-buffer settings. Re-published on every
        // settings save (see `pref::save`), so the knobs apply live.
        compositor_model_environment_background_base::base::set(
            prefs.background_triple_buffer.normalized(),
        );
        // Keyboard-shortcut overrides loaded fresh from keybinding.json.
        let keybinding = compositor_model_environment_keybinding_base::base::load();

        // Audio/media are driver data: stored in the kernel/driver storage by
        // token, not as Orchestrator fields.
        kernel_data.insert(&compositor_orchestration_driver_audio_base::base::AUDIO, AudioController::new("y5.compositor").ok());
        kernel_data.insert(&compositor_orchestration_driver_audio_base::base::MEDIA, Some(MediaController::new()));
        // Introspection driver: sampler slot, populated by the loader post-init.
        kernel_data.insert(&compositor_orchestration_driver_introspection_base::base::SAMPLER, None);
        // Remote driver: RPC state (broadcast + incoming buffer).
        kernel_data.insert(&compositor_orchestration_driver_remote_base::base::RPC, compositor_remote_message_state_base::state::State::new(rpc_broadcast));
        // GPU driver: DRM binding slot, populated post-init by the backend.
        kernel_data.insert(&GPU_BINDING, None);
        // Resume driver: vblank-seen flag + resume watchdog.
        kernel_data.insert(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN, false);
        kernel_data.insert(&compositor_orchestration_driver_resume_base::base::RESUME_WATCHDOG, None);
        // Lid/display driver: kernel-written display snapshot + rim-written request.
        kernel_data.insert(&compositor_orchestration_driver_lid_base::base::DISPLAY_SNAPSHOT, Default::default());
        kernel_data.insert(&compositor_orchestration_driver_lid_base::base::LID_POSITION, None);
        kernel_data.insert(&compositor_orchestration_driver_lid_base::base::DISPLAY_REQUEST, None);
        kernel_data.insert(&compositor_orchestration_driver_lid_base::base::DISPLAY_OFF, false);
        // logind driver: power client, populated post-init by the backend.
        kernel_data.insert(&compositor_orchestration_driver_logind_base::base::LOGIND, None);
        // Capture driver: registry + session state.
        kernel_data.insert(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY, None);
        kernel_data.insert(&compositor_orchestration_driver_capture_base::base::CAPTURE, compositor_y5_graphic_capture_session::session::CaptureState::idle());
        // Backend kind (nested winit vs udev) mirrored into the kernel store so
        // input/draw systems can read it via `cx.kernel`.
        kernel_data.insert(&compositor_orchestration_storage_state_base::state::NESTED, nested);

        // Overview overlay: the session-wide last-active tab (the overview slot
        // itself is per-world; only the tab preference crosses worlds).
        kernel_data.insert(
            &compositor_y5_overview_state_base::base::OVERVIEW_TAB,
            compositor_y5_overview_state_base::base::Tab::Layout,
        );

        // On-screen-keyboard driver state (shown/pinned/mods/placement).
        kernel_data.insert(&compositor_y5_osk_board_state::state::OSK, Default::default());

        // Output-mode driver: rim-issued mode request + kernel-written advertised
        // modes snapshot and apply result (settings window ↔ DRM, like the lid).
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::OUTPUT_MODE_REQUEST, None);
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::OUTPUT_MODES_SNAPSHOT, Default::default());
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::OUTPUT_MODE_RESULT, None);
        // Kernel-written full connector list (the settings Display panel's monitor
        // picker + advertised modes).
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::OUTPUTS_SNAPSHOT, Default::default());
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::TOUCH_DEVICES_SNAPSHOT, Default::default());
        // Rim→kernel: request a reconcile pass after an activate/deactivate.
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::OUTPUT_RECONCILE_REQUEST, false);
        // Baseline of a provisional activate/deactivate awaiting the "check changes"
        // confirm/revert gate (auto-reverts on timeout). `None` until the first toggle.
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::OUTPUT_ACTIVE_REVERT, None);
        // Cursor-teleport layout + current placement (output-arrangement state; not on
        // the Orchestrator). Seeded from prefs with no connected outputs yet — the
        // kernel's first `reconcile` rebuilds it with the connected set.
        kernel_data.insert(
            &compositor_orchestration_driver_output_base::base::TELEPORT_LAYOUT,
            compositor_orchestration_driver_output_base::base::build_teleport(&prefs, &[]),
        );
        kernel_data.insert(&compositor_orchestration_driver_output_base::base::CURSOR_PLACEMENT, None);

        // Settings-window driver: the open/handle/dirty state for the Super+. surface.
        kernel_data.insert(&compositor_orchestration_driver_settings_base::base::SETTINGS, Default::default());

        Self {
            environment,
            render_target: None,
            render_output: None,
            frame_serial: Default::default(),
            cursor_output: None,
            saved_cursor: None,
            finger_scroll_ramp: FingerScrollRamp::default(),
            lock_engage: false,
            control_ping: None,
            __set_picker: None,
            status_session: StatusSession::Active,
            gesture: Default::default(),
            touch: Default::default(),
            storage: compositor_orchestration_storage_state_base::state::Storage::new(nested),
            status: Status::Running,
            start_time,
            loader,
            kernel: kernel_data,
            worlds,
            world_focus_memory: std::collections::HashMap::new(),
            bus: compositor_orchestration_bus_legacy_base::legacy::LegacyBus::new(),
            pilot_tick: 0,
            last_on_pane_awake: std::collections::HashMap::new(),
            suspended: std::collections::HashSet::new(),
            // Seed the live preference object from preferences.json (one disk read
            // at startup; refreshed on each settings-window open). Missing file →
            // sane defaults.
            preference: prefs,
            keybinding,
        }
    }

    /// The window `Space` hosted by the spawn-target spatial world (currently
    /// the main world). Space is owned by the world (document/ARCHITECTURE.md →
    /// "Window tracking"); this is the driver-side accessor. Borrows only
    /// `self` (Orchestrator/`inner`), so it stays disjoint from `Wire.state`.
    /// (WT2 generalizes "main" to the tracked spawn-target.)
    /// Reassign the spawn-target world and, when it actually changes, announce
    /// `WORLD_SWITCHED` so the foreign-toplevel mirror re-advertises the now-hosted
    /// world's windows. Use this instead of `self.worlds.set_spawn_target` directly.
    pub fn set_spawn_target_world(&mut self, id: uuid::Uuid) {
        let previous = self.worlds.spawn_target();
        if self.worlds.set_spawn_target(id) {
            // The outgoing world's off-thread background panes are now nobody's:
            // per-pane targets, a `history` image and a persistent ping-pong pair
            // are the largest thing that feature owns, and without a deterministic
            // signal they sit until the worker's 30s backstop. Passing through the
            // picker touches several worlds in seconds, so that is hundreds of
            // megabytes of fullscreen dmabuf held for nothing.
            //
            // GUARDED ON `active_id`, and this is the load-bearing part. A world
            // that is no longer the spawn target but IS still active is still on
            // screen. Retiring on `WorldManager::switch` instead would free the
            // session world's ring the moment the lock screen took over — the very
            // ring the lock screen then samples through the spawn-target fallback.
            // Lock never calls this function at all, which is why the session world
            // survives it; the guard covers the rest.
            if previous != self.worlds.active_id() {
                compositor_kernel_graphic_bridge_publish_retire::retire::retire_world(
                    previous.as_u128(),
                );
                self.release_world_surfaces(previous);
            }
            self.bus.send(&WORLD_SWITCHED_TX, WorldSwitched);
        }
    }

    /// Release the iced GPU backings of a world that is no longer on screen.
    ///
    /// The same argument as the pane retirement above, for the other per-world
    /// pool. `IcedRegistry::manage_backings` — the thing that frees a hidden
    /// surface's dmabuf — is called once per frame on the registry resolved
    /// through `surface_mut()`, i.e. the SPAWN TARGET's. Every other world's
    /// registry is never passed to it, so its surfaces stay resident: visit a
    /// world, allocate its placeholders and chrome, leave, and that memory is
    /// held for the life of the session. Tens of MiB per world in practice —
    /// worth reclaiming, but not the dominant term in any measurement.
    ///
    /// Called under the same `previous != active_id()` guard: a world that is
    /// not the spawn target but IS still active is on screen and must keep its
    /// backings.
    fn release_world_surfaces(&mut self, world: uuid::Uuid) {
        // Honour the same preference `manage_backings` does — with
        // `release_hidden_surfaces` off, nothing is ever released anywhere.
        if !self.preference.release_hidden_surfaces || !self.worlds.contains(world) {
            return;
        }
        let Some(surface) = self
            .worlds
            .get_mut(world)
            .storage_mut()
            .try_get_mut(&compositor_y5_surface_system_base::base::SURFACE_MUT)
        else {
            return;
        };
        let Some(registry) = surface.registry.as_mut() else {
            return;
        };
        let freed = registry.release_all_backings();
        if freed > 0 {
            info!("world {world}: released {freed} iced backing(s) — left the screen");
        }
    }

    /// The world whose Space contains `window`, if any (used by cross-world foreign
    /// activation to switch to a window that lives on another world).
    pub fn world_of_window(&self, window: &smithay::desktop::Window) -> Option<uuid::Uuid> {
        self.worlds.ids().into_iter().find(|&id| {
            self.worlds
                .get(id)
                .storage()
                .try_get(&compositor_support_world_host_space_base::base::SPACE)
                .map(|w| w.inner.state.elements().any(|e| e == window))
                .unwrap_or(false)
        })
    }

    /// Make `id` both the active AND the spawn-target world (a full switch), enabling
    /// the incoming world and disabling the outgoing one, and announcing `WORLD_SWITCHED`.
    pub fn switch_to_world(&mut self, id: uuid::Uuid) {
        self.worlds.switch(id, &self.kernel);
        self.set_spawn_target_world(id);
    }

    /// Map `output` into EVERY world's Space, not just the hosted one.
    ///
    /// Hotplug is a fact about the machine, but a world only ever learns about it while
    /// it is HOSTED: the plain `space_state_mut()` route resolves to the spawn target, so
    /// a monitor plugged in or pulled out while a world was parked never reached that
    /// world's Space. It came back holding a dead `Output` and missing the live one —
    /// which is load-bearing, because `active_output` / `current_output` pick from that
    /// same list and [`Self::refresh_space`] sends `wl_output` enter/leave over it. Swap
    /// every monitor while a world is parked and its windows come back entered on
    /// nothing but a monitor that is gone.
    ///
    /// A world's Space keeps its OWN location for the output (smithay keys that per
    /// space), so this sets the same layout in each. Overlay worlds hold no Space and
    /// are skipped. A world created LATER still starts empty and is seeded on first
    /// entry by the world-switch paths.
    ///
    /// The enter is published HERE, per world, rather than left to
    /// [`Self::refresh_space`]. Hotplug is the only thing that can change the answer for
    /// a parked world, so paying for it at the event costs one pass per monitor change
    /// instead of a per-world pass on every frame forever.
    pub fn map_output_everywhere(
        &mut self,
        output: &smithay::output::Output,
        location: smithay::utils::Point<i32, smithay::utils::Logical>,
    ) {
        self.for_each_space(|space| {
            space.map_output(output, location);
            space.refresh_outputs_with(|_window, _output| Some(unclipped()));
        });
    }

    /// Unmap `output` from EVERY world's Space — the counterpart of
    /// [`Self::map_output_everywhere`]; see it for why this is not the hosted world only.
    ///
    /// The refresh is what tells the clients: `Space::unmap_output` only drops the output
    /// from the list, and the retain pass inside `refresh_outputs_with` is what turns
    /// that into `wl_surface.leave`.
    pub fn unmap_output_everywhere(&mut self, output: &smithay::output::Output) {
        self.for_each_space(|space| {
            space.unmap_output(output);
            space.refresh_outputs_with(|_window, _output| Some(unclipped()));
        });
    }

    /// Run `f` against every world's window Space. Overlay worlds hold none and are
    /// skipped.
    fn for_each_space(&mut self, mut f: impl FnMut(&mut smithay::desktop::Space<Window>)) {
        for id in self.worlds.ids() {
            if let Some(host) = self
                .worlds
                .get_mut(id)
                .storage_mut()
                .try_get_mut(&compositor_support_world_host_space_base::base::SPACE_MUT)
            {
                f(&mut host.inner.state);
            }
        }
    }

    /// Publish `xdg_toplevel.suspended` across every world, from the per-pane `on_pane_awake`
    /// marker every monitor filled this frame.
    ///
    /// The protocol calls it "surface repaint is suspended" and names occlusion as the
    /// worked example — which is what y5 has been doing all along without saying so,
    /// since a window that draws no pixels is left out of `visible_window` and so gets
    /// no frame callbacks. A client paced on those simply stops, with nothing to tell
    /// it why. This is that fact made sayable.
    ///
    /// Occlusion deliberately does NOT suspend, which is where this parts company with
    /// the protocol's own worked example. The example assumes a stacking desktop, where
    /// being covered is a state the user put the window into; here the occluder is a
    /// sibling on a pannable canvas that can move, close or scroll away in one frame
    /// with nothing to announce it. So `suspended` here means off every pane of every
    /// monitor — parked world, collapsed group, screen locked, or scrolled away — which
    /// are the protocol's other three examples.
    ///
    /// The union across monitors is the whole point of reading `output_views` rather
    /// than one `Viewports`: a window shown on one screen and covered on another is
    /// awake. Every WORLD too, because a parked world's windows are drawn nowhere and
    /// that is the truth to tell them.
    ///
    /// Set lazily and cleared eagerly. [`SUSPEND_DWELL`] of unbroken absence to set,
    /// because a pan drags windows through occlusion continuously and every crossing
    /// would otherwise be a configure; one sighting to clear, because a suspended
    /// client may have stopped drawing entirely and this configure is what wakes it.
    /// Edge-triggered against [`Self::suspended`], so steady state is one set lookup per
    /// window and no protocol work at all.
    pub fn refresh_suspended(&mut self) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
        let now = compositor_pipeline_abi_clock_base::base::now();
        let on_pane_awake: std::collections::HashSet<uuid::Uuid> = self
            .output_views()
            .map
            .values()
            .flat_map(|vps| vps.on_pane_awake.values())
            .flatten()
            .copied()
            .collect();
        for uuid in &on_pane_awake {
            self.last_on_pane_awake.insert(*uuid, now);
        }
        let mut live: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
        for id in self.worlds.ids() {
            let Some(world) = self
                .worlds
                .get(id)
                .storage()
                .try_get(&compositor_support_world_host_space_base::base::SPACE)
            else {
                continue;
            };
            for w in world.inner.state.elements() {
                use compositor_y5_window_interface_record::window::LoopWindow;
                let Some(uuid) = w.uuid() else { continue };
                live.insert(uuid);
                let Some(toplevel) = w.toplevel() else { continue };
                // A window nothing has ever stamped counts its absence from now, so one
                // spawned into a parked world or a collapsed group suspends on the same
                // terms as one that scrolled away rather than instantly.
                let since = *self.last_on_pane_awake.entry(uuid).or_insert(now);
                let suspend = now - since >= SUSPEND_DWELL;
                // EDGE-triggered. `with_pending_state` is not free even when the closure
                // changes nothing: it takes the surface's lock and materialises a
                // `server_pending` clone of the whole state set, which `send_pending_configure`
                // then has to diff and discard. Running that for every window of every
                // world on every frame, to re-assert a flag that flips a few times a
                // session, is the sort of cost that only shows up under a hundred windows.
                if suspend == self.suspended.contains(&uuid) {
                    continue;
                }
                toplevel.with_pending_state(|s| match suspend {
                    true => s.states.set(xdg_toplevel::State::Suspended),
                    false => s.states.unset(xdg_toplevel::State::Suspended),
                });
                // Clients below xdg_wm_base v6 never see the state — smithay filters it
                // per client version (`into_filtered_states`) — so nothing to gate here.
                toplevel.send_pending_configure();
                match suspend {
                    true => self.suspended.insert(uuid),
                    false => self.suspended.remove(&uuid),
                };
            }
        }
        // Keyed by uuid rather than held in the window's user data, so both have to be
        // swept: a window that closed while its world was parked would otherwise leave
        // entries behind for the life of the session.
        self.last_on_pane_awake.retain(|uuid, _| live.contains(uuid));
        self.suspended.retain(|uuid| live.contains(uuid));
    }

    /// Set the `activated` xdg state exclusively on `keep` — true on it, false on every
    /// other mapped window across ALL worlds — sending each a pending configure. Pass
    /// `None` to deactivate everything (e.g. switching into a world with no remembered
    /// focus). `send_pending_configure` is a no-op where nothing changed, so only the
    /// windows whose flag actually flips emit traffic. Enforces exactly-one-activated:
    /// leaving a world never deactivates its windows, so without clearing across every
    /// world a cross-world activation leaves stale `activated=true` flags in other
    /// worlds. That stale flag makes re-activating the target a no-op diff the foreign
    /// mirror never forwards, so a dock (sfwbar) never learns the target became focused.
    pub fn set_activated_exclusive(&self, keep: Option<&smithay::desktop::Window>) {
        for id in self.worlds.ids() {
            let Some(world) = self
                .worlds
                .get(id)
                .storage()
                .try_get(&compositor_support_world_host_space_base::base::SPACE)
            else {
                continue;
            };
            for w in world.inner.state.elements() {
                w.set_activated(Some(w) == keep);
                if let Some(toplevel) = w.toplevel() {
                    toplevel.send_pending_configure();
                }
            }
        }
    }

    pub fn space_state(&self) -> &compositor_support_smithay_state_space_base::state::SpaceState {
        let target = self.worlds.spawn_target();
        &self
            .worlds
            .get(target)
            .storage()
            .get(&compositor_support_world_host_space_base::base::SPACE)
            .inner
    }

    pub fn space_state_mut(&mut self) -> &mut compositor_support_smithay_state_space_base::state::SpaceState {
        let target = self.worlds.spawn_target();
        &mut self
            .worlds
            .get_mut(target)
            .storage_mut()
            .get_mut(&compositor_support_world_host_space_base::base::SPACE_MUT)
            .inner
    }

    /// The space of every world EXCEPT the hosted (spawn-target) one. Those
    /// worlds' windows are invisible by definition (nothing renders them), so
    /// the per-window fractional scale can publish scale 1 to them under the
    /// `fractional_invisible` optimized/full strategies.
    pub fn other_world_spaces(&self) -> Vec<&compositor_support_smithay_state_space_base::state::SpaceState> {
        let hosted = self.worlds.spawn_target();
        self.worlds
            .ids()
            .into_iter()
            .filter(|&id| id != hosted)
            .filter_map(|id| {
                self.worlds
                    .get(id)
                    .storage()
                    .try_get(&compositor_support_world_host_space_base::base::SPACE)
            })
            .map(|w| &w.inner)
            .collect()
    }

    /// The [`OutputKey`](compositor_orchestration_driver_output_base::base::OutputKey)
    /// of the output the focus accessors resolve against: the one being rendered
    /// (`render_output`, inside the per-output render loop), else the one under the
    /// cursor (`cursor_output`), else `""` (the sole / not-yet-identified output,
    /// whose bootstrap view tree is always present).
    /// Bump and return this output's monotonic scene-build counter.
    ///
    /// On the native path `execute()` runs once per vblank per output, so this is
    /// a retrace count — which is what the off-thread background's vblank cadence
    /// paces on. Per output, not global: with two monitors a single counter would
    /// advance once per output per retrace and silently halve every divisor.
    /// Under winit it degrades to a plain frame counter, as that cadence expects.
    pub fn next_frame_serial(&mut self) -> u64 {
        let key = self.current_output_key();
        let slot = self.frame_serial.entry(key).or_default();
        *slot = slot.wrapping_add(1);
        *slot
    }

    /// Refresh interval of the output being drawn. Falls back to 30Hz before a
    /// mode is known; `Rate::Multiplier` is relative to this. The fallback is
    /// deliberately SLOW: consumers turn it into a minimum interval, so a high
    /// guess would uncap them until the real mode arrives.
    pub fn current_refresh(&self) -> std::time::Duration {
        self.current_output()
            .current_mode()
            .filter(|m| m.refresh > 0)
            .map(|m| std::time::Duration::from_secs_f64(1000.0 / m.refresh as f64))
            .unwrap_or_else(|| std::time::Duration::from_micros(33_333))
    }

    /// The SHORTEST refresh interval among mapped outputs — the fastest panel.
    ///
    /// For the UI pacing global, which is one value for the whole desktop
    /// because the worker has no output of its own to ask.
    ///
    /// Fastest, and this is not a compromise — it follows from where bevy
    /// elements are drawn. They are pushed at `layer::WORLD_3D` and are NOT
    /// gated by `draw_screen`, so a bevy instance is composited on EVERY mapped
    /// output. A surface genuinely shown on a 144Hz panel must not be paced to
    /// 60 because a second monitor is slower; that is visible stutter on the
    /// monitor the user is looking at.
    ///
    /// And it costs nothing on the slow panel, because the ceiling is not what
    /// bounds it there — the compositor composites each output at that output's
    /// own rate, so the 60Hz monitor sees 60 frames whatever this says. The
    /// ceiling's job is only to stop the producer outrunning what ANY panel can
    /// show, and the fastest panel is exactly that bound.
    ///
    /// If bevy elements are ever gated per output, this stops being right and
    /// the fix is a per-instance refresh rather than a different reduction here:
    /// the correct value is the fastest among the outputs showing THAT instance.
    pub fn fastest_refresh(&self) -> std::time::Duration {
        self.space_state()
            .state
            .outputs()
            .filter_map(|o| o.current_mode())
            .filter(|m| m.refresh > 0)
            .map(|m| std::time::Duration::from_secs_f64(1000.0 / m.refresh as f64))
            .min()
            .unwrap_or_else(|| std::time::Duration::from_micros(33_333))
    }

    pub fn current_output_key(&self) -> compositor_orchestration_driver_output_base::base::OutputKey {
        self.render_output
            .clone()
            .or_else(|| self.cursor_output.clone())
            .unwrap_or_default()
    }

    /// The ACTIVE output's key: the one under the cursor (`cursor_output`), else the
    /// primary/first. Unlike [`current_output_key`](Self::current_output_key) this
    /// IGNORES `render_output` — screen-space surfaces (launcher/settings/menu) live
    /// on the monitor the user is on, not on whichever output the render loop is
    /// currently drawing. `None` until the pointer resolves an output.
    pub fn active_output_key(&self) -> compositor_orchestration_driver_output_base::base::OutputKey {
        self.cursor_output.clone().unwrap_or_else(|| {
            self.space_state()
                .state
                .outputs()
                .next()
                .map(output_key)
                .unwrap_or_default()
        })
    }

    /// The smithay `Output` the user is on (cursor's output, else primary) — the
    /// target + size source for screen-space surfaces.
    pub fn active_output(&self) -> &smithay::output::Output {
        let key = self.active_output_key();
        let space = self.space_state();
        space
            .state
            .outputs()
            .find(|o| output_key(o) == key)
            .or_else(|| space.state.outputs().next())
            .expect("at least one mapped output")
    }

    /// The smithay `Output` matching [`current_output_key`](Self::current_output_key),
    /// falling back to the first mapped output (so single-output paths are unchanged).
    pub fn current_output(&self) -> &smithay::output::Output {
        let key = self.current_output_key();
        let space = self.space_state();
        space
            .state
            .outputs()
            .find(|o| output_key(o) == key)
            .or_else(|| space.state.outputs().next())
            .expect("at least one mapped output")
    }

    /// Wake the native control-plane ping so the display request queues drain on
    /// the next loop iteration. Call after queuing a mode/switch/lid request. A
    /// no-op on winit (no ping registered), where those requests don't apply.
    pub fn ping_control(&self) {
        if let Some(ping) = &self.control_ping {
            ping.ping();
        }
    }

    /// FOCUS ACCESSOR (document/WORLD_DELEGATION.md): the camera/viewport of the
    /// focused world, for the CURRENT output — each monitor is its own viewport with
    /// its own camera. Resolves the current output's `Viewports` (render output while
    /// drawing, else cursor output), then the pane within it.
    pub fn camera(&self) -> &compositor_y5_camera_state_base::state::Camera {
        let target = self.worlds.spawn_target();
        let key = self.current_output_key();
        let viewports = self.worlds.get(target).storage().get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS).views(&key);
        // Inside the per-region render loop, resolve the pane being drawn; else
        // the focused (active) slot.
        match self.render_target {
            Some(rt) => viewports.camera_of(rt.slot).unwrap_or_else(|| viewports.focus_camera()),
            None => viewports.focus_camera(),
        }
    }

    pub fn camera_mut(&mut self) -> &mut compositor_y5_camera_state_base::state::Camera {
        let target = self.worlds.spawn_target();
        let key = self.current_output_key();
        let render_slot = self.render_target.map(|rt| rt.slot);
        let viewports = self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS_MUT).views_mut(&key);
        // Render target may be a floating pane's slot, so search all panes.
        match render_slot.filter(|id| viewports.camera_of(*id).is_some()) {
            Some(id) => viewports.camera_of_mut(id).expect("checked present"),
            None => viewports.focus_camera_mut(),
        }
    }

    /// The ACTIVE output's focused camera: the monitor the user is on (cursor's
    /// output, else primary), IGNORING `render_output`. Unlike [`camera`](Self::camera)
    /// — which resolves against `current_output_key` and so returns whichever output
    /// the per-output render loop is currently DRAWING — this follows the user's
    /// monitor. Use it for spawn-time placement (new windows must land on the active
    /// screen, not the one whose render pass happens to be draining the map queue).
    /// Mirrors [`active_output`](Self::active_output) for screen-space surfaces.
    pub fn active_camera(&self) -> &compositor_y5_camera_state_base::state::Camera {
        let target = self.worlds.spawn_target();
        let key = self.active_output_key();
        self.worlds
            .get(target)
            .storage()
            .get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS)
            .views(&key)
            .focus_camera()
    }

    /// FOCUS ACCESSOR: the CURRENT output's viewport tree (slots + cameras).
    pub fn viewports(&self) -> &compositor_y5_viewport_state_base::state::Viewports {
        let target = self.worlds.spawn_target();
        let key = self.current_output_key();
        self.worlds.get(target).storage().get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS).views(&key)
    }

    pub fn viewports_mut(&mut self) -> &mut compositor_y5_viewport_state_base::state::Viewports {
        let target = self.worlds.spawn_target();
        let key = self.current_output_key();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS_MUT).views_mut(&key)
    }

    /// The per-output view map. Used to select/create the current output's view tree
    /// (the render loop ensures each drawn output has its own `Viewports`; the
    /// pointer path points `current` at the cursor's output for the systems).
    pub fn output_views_mut(&mut self) -> &mut compositor_y5_viewport_state_base::state::OutputViews {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS_MUT)
    }

    /// Read-only per-output view map — every output's `Viewports` (cameras + visible
    /// sets). Used to derive cross-output state (e.g. a window's best-resolution
    /// fractional scale = highest zoom of any viewport across ALL outputs showing it).
    pub fn output_views(&self) -> &compositor_y5_viewport_state_base::state::OutputViews {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_viewport_state_base::state::OUTPUT_VIEWS)
    }

    /// The Space refresh, with `wl_output` membership as y5 actually models it: every
    /// window is on every output.
    ///
    /// Stands in for `Space::refresh()`, whose `refresh_outputs` third decides which
    /// outputs a window is on by intersecting the window's STORED position with the
    /// output's geometry. That is right for a compositor whose Space is the screen, and
    /// meaningless here twice over.
    ///
    /// First the position is a y5-WORLD coordinate — the camera is applied at render
    /// time, see `document/TRANSFORM.md` — while the outputs are mapped into that same
    /// Space from the origin. A window at negative world x or y intersects no output and
    /// is sent `wl_surface.leave`; a client that gates its render loop on being on an
    /// output then stops drawing, frozen from its first frame and released only by
    /// dragging it back over the origin. With several monitors it is worse than wrong —
    /// the outputs tile side by side, so a window's world x picks whose scale and refresh
    /// the client renders for.
    ///
    /// Second — and this is why the answer is a constant rather than a better geometric
    /// test — no window BELONGS to an output. Every monitor renders the same world
    /// through its own camera, so any monitor can bring any window into view on the next
    /// frame, with no commit and no protocol event to hang a re-entry off. There is
    /// nothing to test: the truthful `enter` set is every mapped output, and the only
    /// `leave` is an output going away, which the retain pass inside
    /// `refresh_outputs_with` still does.
    ///
    /// This deliberately does NOT double as a visibility signal. A window outside every
    /// camera is still ON its outputs — it is one pan away, and nothing about it has
    /// changed. `wl_output` membership is a rendering contract, and a client may stop
    /// drawing entirely on `leave`, so driving it from an off-screen test is a far
    /// blunter instrument than the one y5 already aims at invisible windows
    /// (`fractional_invisible`, which only decides what scale to publish and defaults to
    /// "keep publishing"). Only one mechanism gets to decide whether a client renders at
    /// all, and it should not be this one. The foreign-toplevel mirror answers the same
    /// question the same way, so a dock and the client are never told different things.
    ///
    /// Applies to EVERY element of the space and, through `output_update`, to every
    /// surface of each one's tree — subsurfaces and popups included. There is no
    /// toplevel filter: a `wl_output` is entered by surfaces, not by windows. (Layer
    /// surfaces are not space elements; `LayerMap::arrange` drives theirs, and for those
    /// the geometric test IS right — they are screen-space by definition.)
    ///
    /// Only the alive sweep runs over every world; the enter/leave pass and the element
    /// refresh are the hosted world's alone. The split is by what can actually change:
    ///
    /// - `refresh_alive` is the ONLY reaper of a dead window — there is no `unmap_elem`
    ///   on window destroy anywhere (only whole-world delete), so a client that exits
    ///   while its world is parked would sit in that world's Space until the world came
    ///   back, and with `protocol_foreign_all_worlds` the mirror walks every world and
    ///   would hand docks a dead toplevel. A `retain` over a short Vec, so it is cheap
    ///   enough to do for all of them every frame.
    /// - Membership is a constant, so for a parked world only HOTPLUG can change the
    ///   answer — and [`Self::map_output_everywhere`] publishes it there, per world, at
    ///   the event. Doing it here as well would be one pass per world per frame to
    ///   re-derive something that changes a few times a session.
    /// - `refresh_elements` is the RE-ASSERTION pass, walking every window's whole
    ///   surface tree to re-send a decision that has not changed. The decision itself is
    ///   delivered inline — `output_enter` ends by refreshing the element it just
    ///   entered, and `output_leave` sends its own leaves — so a parked world is still
    ///   owed nothing.
    pub fn refresh_space(&mut self) {
        self.for_each_space(|space| space.refresh_alive());
        let space = &mut self.space_state_mut().state;
        space.refresh_outputs_with(|_window, _output| Some(unclipped()));
        space.refresh_elements();
    }

    /// Cursor teleportation between monitors is currently suppressed: some system holds
    /// the [`TELEPORT_SUPPRESS`] lock (refcount > 0) to pin the cursor to its output — a
    /// canvas pan is the built-in client. Read by the relative-motion path; it knows
    /// nothing about WHY (which operation raised the lock). Reads the spawn-target world's
    /// storage, exactly like [`canvas`](Self::canvas).
    pub fn teleport_suppressed(&self) -> bool {
        let target = self.worlds.spawn_target();
        *self
            .worlds
            .get(target)
            .storage()
            .get(&compositor_orchestration_driver_output_base::base::TELEPORT_SUPPRESS)
            > 0
    }

    /// FOCUS ACCESSOR: the focused world's canvas slot (input grab, …).
    pub fn canvas(&self) -> &compositor_y5_canvas_state_base::state::CanvasState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_canvas_system_base::base::CANVAS)
    }

    pub fn canvas_mut(&mut self) -> &mut compositor_y5_canvas_state_base::state::CanvasState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_canvas_system_base::base::CANVAS_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's navigator state machine.
    pub fn navigator(&self) -> &compositor_y5_navigator_state_base::state::Machine {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_navigator_state_base::state::NAVIGATOR)
    }

    pub fn navigator_mut(&mut self) -> &mut compositor_y5_navigator_state_base::state::Machine {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_navigator_state_base::state::NAVIGATOR_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's channel router — rim triggers announce
    /// here so the focused world's systems receive (replaces get(MAIN_WORLD).channels()).
    pub fn focus_channels(&mut self) -> &mut compositor_support_system_channel_router_base::base::ChannelRouter {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).channels()
    }

    /// FOCUS ACCESSOR: the focused world's window-selection slot.
    pub fn select(&self) -> &compositor_y5_select_state_base::select::CanvasSelect {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_select_state_base::select::SELECT)
    }

    pub fn select_mut(&mut self) -> &mut compositor_y5_select_state_base::select::CanvasSelect {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_select_state_base::select::SELECT_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's settings-panel surface. Per-world for the
    /// same reason as the toolbar below; the rest of `SETTINGS` is session-wide and
    /// stays in the kernel store. See `SETTINGS_SURFACE`.
    pub fn settings_surface(&self) -> compositor_orchestration_driver_settings_base::base::SettingsSurface {
        let target = self.worlds.spawn_target();
        *self.worlds.get(target).storage().get(&compositor_orchestration_driver_settings_base::base::SETTINGS_SURFACE)
    }

    pub fn settings_surface_mut(&mut self) -> &mut compositor_orchestration_driver_settings_base::base::SettingsSurface {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_orchestration_driver_settings_base::base::SETTINGS_SURFACE_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's align/distribute toolbar slot. Per-world
    /// because it stores handles into `surface()`'s registry AND is reconciled
    /// against `select()`, both of which are per-world — all three must resolve to
    /// the same world or a switch strands the toolbar in the world that built it.
    pub fn selection_overlay(
        &self,
    ) -> &compositor_orchestration_driver_selection_base::base::SelectionOverlayState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_orchestration_driver_selection_base::base::SELECTION_OVERLAY)
    }

    pub fn selection_overlay_mut(
        &mut self,
    ) -> &mut compositor_orchestration_driver_selection_base::base::SelectionOverlayState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_orchestration_driver_selection_base::base::SELECTION_OVERLAY_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's window-grouping slot.
    pub fn group(&self) -> &compositor_y5_group_state_base::state::GroupState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_group_state_base::state::GROUP)
    }

    pub fn group_mut(&mut self) -> &mut compositor_y5_group_state_base::state::GroupState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_group_state_base::state::GROUP_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's surface slot (iced registry + the
    /// surface-message channel). Per-world; the shared iced GPU context being
    /// wired only to the main world is a separate concern (a test world's
    /// registry is None until that lands).
    pub fn surface(&self) -> &compositor_y5_surface_state_base::state::SurfaceState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_surface_system_base::base::SURFACE)
    }

    pub fn surface_mut(&mut self) -> &mut compositor_y5_surface_state_base::state::SurfaceState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_surface_system_base::base::SURFACE_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's guide-popup slot (the empty-canvas
    /// context menu, the help panel and the inline shader editor). Per-world
    /// because it stores handles into `surface()`'s registry, which is per-world —
    /// the two must resolve to the same world or a switch strands the surfaces.
    pub fn guide(&self) -> &compositor_y5_guide_state_base::state::GuideState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_guide_state_base::state::GUIDE)
    }

    pub fn guide_mut(&mut self) -> &mut compositor_y5_guide_state_base::state::GuideState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_guide_state_base::state::GUIDE_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's overview-mode slot (Super+Tab overlay).
    pub fn overview(&self) -> &compositor_y5_overview_state_base::base::Overview {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_overview_state_base::base::OVERVIEW)
    }

    pub fn overview_mut(&mut self) -> &mut compositor_y5_overview_state_base::base::Overview {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_overview_state_base::base::OVERVIEW_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's pointer slot (cursor world coords).
    pub fn pointer(&self) -> &compositor_orchestration_seat_pointer_state::state::PointerState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_orchestration_seat_system_pointer::base::POINTER)
    }

    pub fn pointer_mut(&mut self) -> &mut compositor_orchestration_seat_pointer_state::state::PointerState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_orchestration_seat_system_pointer::base::POINTER_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's placeholder slot.
    pub fn placeholder(&self) -> &compositor_y5_placeholder_state_base::state::PlaceholderState {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_placeholder_system_base::base::PLACEHOLDER)
    }

    pub fn placeholder_mut(&mut self) -> &mut compositor_y5_placeholder_state_base::state::PlaceholderState {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_placeholder_system_base::base::PLACEHOLDER_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's launcher slot.
    pub fn launcher(&self) -> &compositor_y5_launcher_draw_state::state::State {
        let target = self.worlds.spawn_target();
        self.worlds.get(target).storage().get(&compositor_y5_launcher_system_base::base::LAUNCHER)
    }

    pub fn launcher_mut(&mut self) -> &mut compositor_y5_launcher_draw_state::state::State {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_launcher_system_base::base::LAUNCHER_MUT)
    }

    /// FOCUS ACCESSOR: the focused world's window-lifecycle queue (smithay Wire
    /// pushes map/destroy/fullscreen events here; the focused world's WindowSystem
    /// drains them — new windows map into the focused/spawn-target world).
    pub fn window_lifecycle_mut(&mut self) -> &mut compositor_y5_window_lifecycle_state::lifecycle::WindowLifecycle {
        let target = self.worlds.spawn_target();
        self.worlds.get_mut(target).storage_mut().get_mut(&compositor_y5_window_system_base::base::WINDOW_LIFECYCLE_MUT)
    }

    /// Register a drawable at the top of the draw-order authority. Agnostic:
    /// works for any drawable (windows today; iced surfaces, …). Called from
    /// EVERY map path so `drawable_order()` never drops a live drawable.
    pub fn register_drawable(&mut self, uuid: uuid::Uuid, layer: compositor_support_world_order_track_base::base::DrawLayer) {
        let target = self.worlds.spawn_target();
        self.worlds
            .get_mut(target)
            .storage_mut()
            .get_mut(&compositor_support_world_order_track_base::base::DRAW_ORDER_MUT)
            .insert_top(compositor_support_world_order_track_base::base::ComponentId(uuid), layer);
    }

    /// Raise a drawable to the top of the spatial world's draw-order authority
    /// (windows mirror `space.raise_element`; iced raises on interaction). Lazily
    /// registers if absent. See document/ARCHITECTURE.md → "Window tracking".
    pub fn raise_drawable(&mut self, uuid: uuid::Uuid) {
        let target = self.worlds.spawn_target();
        self.worlds
            .get_mut(target)
            .storage_mut()
            .get_mut(&compositor_support_world_order_track_base::base::DRAW_ORDER_MUT)
            .raise(compositor_support_world_order_track_base::base::ComponentId(uuid));
    }

    /// Hand a drawable's exact draw-order slot (tier + z-position) to a
    /// successor, keeping the predecessor's position instead of the successor
    /// popping to the top (a restored window inheriting the placeholder tile it
    /// maps over). This also GCs the predecessor's now-dead entry. Returns
    /// `false` when `old` wasn't registered, so callers fall back to
    /// `register_drawable`.
    pub fn reassign_drawable(&mut self, old: uuid::Uuid, new: uuid::Uuid) -> bool {
        let target = self.worlds.spawn_target();
        self.worlds
            .get_mut(target)
            .storage_mut()
            .get_mut(&compositor_support_world_order_track_base::base::DRAW_ORDER_MUT)
            .reassign(
                compositor_support_world_order_track_base::base::ComponentId(old),
                compositor_support_world_order_track_base::base::ComponentId(new),
            )
    }

    /// Unregister a drawable from the draw-order authority (event-driven GC):
    /// called from EVERY destruction path (window unmap, iced surface destroy)
    /// so the order never retains a dead component. Foreign/absent ids are a
    /// no-op (`DrawOrder::remove` retains-by-id).
    pub fn remove_drawable(&mut self, uuid: uuid::Uuid) {
        let target = self.worlds.spawn_target();
        self.worlds
            .get_mut(target)
            .storage_mut()
            .get_mut(&compositor_support_world_order_track_base::base::DRAW_ORDER_MUT)
            .remove(compositor_support_world_order_track_base::base::ComponentId(uuid));
    }

    /// Drawable ids in draw order, TOPMOST-FIRST (matches smithay's
    /// first-is-front element order). Each owner resolves its own ids and skips
    /// the rest (the canvas resolves window uuids via the space).
    pub fn drawable_order(&self) -> Vec<uuid::Uuid> {
        let target = self.worlds.spawn_target();
        self.worlds
            .get(target)
            .storage()
            .get(&compositor_support_world_order_track_base::base::DRAW_ORDER)
            .ordered()
            .iter()
            .rev()
            .map(|(id, _)| id.0)
            .collect()
    }

    /// The spawn-target (spatial) world's raw `Storage` — the read source the rim
    /// hit-test bundles into a `HitCx`. A Pass-1 input system gets the equivalent
    /// as `cx.storage` (its active world), so the same hit-test logic serves both.
    pub fn spatial_storage(&self) -> &compositor_support_system_storage_slot_base::base::Storage {
        self.worlds.get(self.worlds.spawn_target()).storage()
    }
}

pub trait CoordinateTrait {
    /// Full-output projection context: anchored to the whole output's centre.
    /// Use for screen-space content (screen iced surfaces, pointer, layer-shell,
    /// lock/overview/picker) — anything that spans the physical screen, NOT an
    /// individual viewport pane. Always full-output, even inside the per-region
    /// render loop.
    fn size_ctx_all(&self) -> compositor_y5_camera_transform_translate::transform::Context;

    /// Projection context for a SPECIFIC viewport pane (`slot`): its camera
    /// anchored to its on-screen region rect, independent of the render loop /
    /// `render_target`. Use anywhere that must be truly per-viewport (e.g.
    /// snapping to a pane's extent). Falls back to full-output if the slot is gone.
    fn size_ctx_viewport(
        &self,
        slot: compositor_y5_viewport_state_base::state::SlotId,
    ) -> compositor_y5_camera_transform_translate::transform::Context;

    /// Per-viewport projection context. Inside the per-region render loop it
    /// anchors to the pane being drawn (its rect + that slot's camera); outside
    /// the loop it equals [`size_ctx_all`]. Use for world content drawn into a
    /// viewport (windows, decorations, canvas cursor, selection) so it projects
    /// into — and is clipped to — the active pane.
    fn viewport_context(&self) -> compositor_y5_camera_transform_translate::transform::Context;

    /// Resolve which viewport pane the physical cursor `phys` is over, record it
    /// as the focused world's `pointer` slot (so `camera()` / camera systems / the
    /// cursor now operate on THAT pane), and return that pane's region context for
    /// mapping the physical cursor to world coordinates. Used by the pointer input
    /// path so input always follows the pane under the cursor, never the
    /// keyboard-`active` pane.
    fn pointer_context(
        &mut self,
        phys: smithay::utils::Point<f64, smithay::utils::Physical>,
    ) -> compositor_y5_camera_transform_translate::transform::Context;

    /// Read-only region context for the CURRENT pointer pane (the `pointer` slot,
    /// already set by the last motion). Use to project the world pointer location
    /// back to physical for the cursor — outside the per-region render loop, where
    /// `viewport_context` would otherwise fall back to full-output.
    fn focus_pane_context(&self) -> compositor_y5_camera_transform_translate::transform::Context;
}
impl CoordinateTrait for Loop {
    fn size_ctx_all(&self) -> compositor_y5_camera_transform_translate::transform::Context {
        let output = self.inner.current_output();
        let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
        let scale = output.current_scale().fractional_scale();
        let camera = &self.inner.camera().transform;
        compositor_y5_camera_transform_translate::transform::Context::new(
            (camera.position.x, camera.position.y),
            camera.zoom,
            (mode.size.w as f64, mode.size.h as f64),
            scale,
        )
    }

    fn size_ctx_viewport(
        &self,
        slot: compositor_y5_viewport_state_base::state::SlotId,
    ) -> compositor_y5_camera_transform_translate::transform::Context {
        let (mode_w, mode_h, scale) = {
            let output = self.inner.current_output();
            let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
            (mode.size.w, mode.size.h, output.current_scale().fractional_scale())
        };
        let bounds = smithay::utils::Rectangle::new(smithay::utils::Point::from((0, 0)), smithay::utils::Size::from((mode_w, mode_h)));
        let viewports = self.inner.viewports();
        // Slot missing → full output.
        let Some(rect) = compositor_y5_viewport_layout_base::layout::compute(viewports, bounds)
            .regions
            .iter()
            .find(|r| r.slot == slot)
            .map(|r| r.rect)
        else {
            return self.size_ctx_all();
        };
        let camera = viewports.camera_of(slot).map(|c| &c.transform).unwrap_or(&viewports.focus_camera().transform);
        compositor_y5_camera_transform_translate::transform::Context::new_region(
            (camera.position.x, camera.position.y),
            camera.zoom,
            (rect.loc.x as f64 / scale, rect.loc.y as f64 / scale),
            (rect.size.w as f64, rect.size.h as f64),
            scale,
        )
    }

    fn viewport_context(&self) -> compositor_y5_camera_transform_translate::transform::Context {
        // No active pane → full output (identical to `size_ctx_all`).
        let Some(rt) = self.inner.render_target else {
            return self.size_ctx_all();
        };
        let output = self.inner.current_output();
        let scale = output.current_scale().fractional_scale();
        // `camera()` already resolves to the render-target pane's slot camera.
        let camera = &self.inner.camera().transform;
        compositor_y5_camera_transform_translate::transform::Context::new_region(
            (camera.position.x, camera.position.y),
            camera.zoom,
            rt.origin_logical,
            rt.size_physical,
            scale,
        )
    }

    fn pointer_context(
        &mut self,
        phys: smithay::utils::Point<f64, smithay::utils::Physical>,
    ) -> compositor_y5_camera_transform_translate::transform::Context {
        let (mode_w, mode_h, scale) = {
            let output = self.inner.current_output();
            let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
            (mode.size.w, mode.size.h, output.current_scale().fractional_scale())
        };
        let bounds = smithay::utils::Rectangle::new(
            smithay::utils::Point::from((0, 0)),
            smithay::utils::Size::from((mode_w, mode_h)),
        );
        let computed = compositor_y5_viewport_layout_base::layout::compute(self.inner.viewports(), bounds);
        let p = smithay::utils::Point::<i32, smithay::utils::Physical>::from((phys.x.round() as i32, phys.y.round() as i32));
        // During a viewport drag (separator / floating move-resize), FREEZE the
        // operative pane so the cursor mapping stays put and can't jump between
        // viewports mid-drag. Otherwise: over a leaf → that pane; over a
        // separator/gap → keep the current pane (so the round-trip still lands on
        // the separator for hit-testing and the cursor doesn't jump).
        let dragging = {
            let views = self.inner.output_views();
            views.separator_drag.is_some() || views.floating_drag.is_some()
        };
        let (slot, rect) = if dragging {
            let current = self.inner.viewports().pointer;
            let rect = computed.regions.iter().find(|reg| reg.slot == current).map(|reg| reg.rect).unwrap_or(bounds);
            (current, rect)
        } else {
            match compositor_y5_viewport_layout_base::layout::slot_at(&computed, p) {
                Some((s, r)) => (s, r),
                None => {
                    let current = self.inner.viewports().pointer;
                    let rect = computed.regions.iter().find(|reg| reg.slot == current).map(|reg| reg.rect).unwrap_or(bounds);
                    (current, rect)
                }
            }
        };
        self.inner.viewports_mut().pointer = slot;
        // `camera()` now resolves to the pointer pane (just set).
        let (cx, cy, cz) = {
            let c = &self.inner.camera().transform;
            (c.position.x, c.position.y, c.zoom)
        };
        compositor_y5_camera_transform_translate::transform::Context::new_region(
            (cx, cy),
            cz,
            (rect.loc.x as f64 / scale, rect.loc.y as f64 / scale),
            (rect.size.w as f64, rect.size.h as f64),
            scale,
        )
    }

    fn focus_pane_context(&self) -> compositor_y5_camera_transform_translate::transform::Context {
        let (mode_w, mode_h, scale) = {
            let output = self.inner.current_output();
            let mode = output.current_mode().unwrap_or_else(|| abort!("output has a current mode"));
            (mode.size.w, mode.size.h, output.current_scale().fractional_scale())
        };
        let bounds = smithay::utils::Rectangle::new(
            smithay::utils::Point::from((0, 0)),
            smithay::utils::Size::from((mode_w, mode_h)),
        );
        let pointer = self.inner.viewports().pointer;
        let computed = compositor_y5_viewport_layout_base::layout::compute(self.inner.viewports(), bounds);
        let rect = computed.regions.iter().find(|r| r.slot == pointer).map(|r| r.rect).unwrap_or(bounds);
        // `camera()` resolves to the pointer pane (focus_camera) outside the render loop.
        let (cx, cy, cz) = {
            let c = &self.inner.camera().transform;
            (c.position.x, c.position.y, c.zoom)
        };
        compositor_y5_camera_transform_translate::transform::Context::new_region(
            (cx, cy),
            cz,
            (rect.loc.x as f64 / scale, rect.loc.y as f64 / scale),
            (rect.size.w as f64, rect.size.h as f64),
            scale,
        )
    }

}

