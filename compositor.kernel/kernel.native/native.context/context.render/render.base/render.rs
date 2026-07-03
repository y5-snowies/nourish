//! The per-pipe render context (moved from udev.draw/draw.context). Owns OUR
//! names for the hosted pipe objects via `scanout.surface/surface.output`,
//! the tap subscriptions the frame executor consults (Law 5: taps fire only
//! for active subscribers), and the live Law-7 safety-net enablement set
//! (seeded from preference, updated through `device.interface`).

use compositor_kernel_scanout_surface_output_base::output::{
    NativeDrmOutput, NativeDrmOutputManager,
};
use compositor_kernel_graphic_draw_plan_tap::tap::TapSubscriptions;
use compositor_kernel_graphic_preference_enable_safety::safety::SafetyEnable;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::output::{Mode, Output};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::drm::control::Mode as DrmMode;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::DisplayHandle;
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::state::StateDRMBinding;

/// One physical output's pipe state: everything scoped to a single monitor (its
/// smithay `Output`, current mode, damage tracker, live scanout target, connector
/// and HDR/mode-revert state). Grouping these into one struct is the boundary the
/// multi-output work builds on: today `NativeRenderContext` holds exactly one
/// `pipe`; multi-monitor turns that into a collection, one entry per lit CRTC. The
/// per-pipe field semantics are unchanged from the single-output era.
pub struct OutputPipe {
    /// The CRTC driving this output — the key that routes a per-CRTC VBlank event
    /// (`wire.frame`) back to the pipe that flipped.
    pub crtc: smithay::reexports::drm::control::crtc::Handle,
    pub mode: Mode,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    /// The live scanout target. `Option` because a live monitor switch tears the
    /// current output DOWN before building the target (single-output hardware can't
    /// light two at once — the atomic modeset of a second output fails). It is
    /// `None` only transiently inside `display.reconcile` between teardown and rebuild;
    /// every render path treats `None` as "skip this frame".
    pub drm_output: Option<NativeDrmOutput>,
    /// HDR (M5): the display's parsed EDID HDR/colorimetry caps.
    pub hdr_caps: compositor_kernel_drm_edid_parse_base::parse::HdrInfo,
    /// HDR output path active this session (`COMPOSITOR_HDR` + capable display +
    /// Vulkan). When true the executor signals the connector (BT.2020 + PQ) once
    /// and composites in the HDR working space.
    pub hdr_active: bool,
    /// Whether the one-time DRM HDR output signalling has been applied.
    pub hdr_signalled: bool,
    pub connector: smithay::reexports::drm::control::connector::Handle,
    /// The mode currently driving the pipe. Seeded at wire time and updated on every
    /// successful live mode change (`display.mode`) — the baseline an auto-revert
    /// restores to.
    pub current_drm_mode: DrmMode,
    /// The connector's advertised modes (from EDID), so the live mode-change drain
    /// can resolve a requested width/height/refresh to a `DrmMode` without
    /// re-probing the connector.
    pub modes: Vec<DrmMode>,
    /// Armed confirm/revert watchdog for a provisionally-applied mode:
    /// `(previous_mode, one_shot_timer)`. `Some` while awaiting the user's Keep;
    /// cleared on Confirm, on Revert, or when the timer reverts. See `display.mode`.
    pub mode_revert: Option<(DrmMode, RegistrationToken)>,
    /// The `wl_output` global for this pipe's Output, when this pipe created its own
    /// (secondary outputs via `display.reconcile::add_output`). Kept so it can be
    /// DESTROYED when the pipe is pruned (disconnect/deactivate) — otherwise a stale
    /// global lingers and re-adding the monitor advertises a duplicate. `None` for the
    /// primary anchor (its global is created once at boot and never pruned).
    pub global: Option<smithay::reexports::wayland_server::backend::GlobalId>,
    /// This pipe has a page-flip in flight (queued, awaiting its own VBlank).
    /// The render loop SKIPS an in-flight pipe so each output re-renders only on
    /// its OWN vblank cadence — a 144 Hz output is not dragged down to a 60 Hz
    /// neighbour's rate by being re-rendered (and CPU-synced) on every vblank of
    /// either output. Set true on a successful queue (`present`), cleared when
    /// this pipe's CRTC delivers its vblank (`wire.frame::process_vblank`) and on
    /// session resume. Single-output behaviour is unchanged (one pipe, its own
    /// vblank clears it every frame).
    pub in_flight: bool,
}

pub struct NativeRenderContext {
    pub display_handle: DisplayHandle,
    /// The lit outputs, one [`OutputPipe`] per driven CRTC. Invariant: NON-EMPTY —
    /// there is always at least one entry (in the single-output / dark eras its
    /// `drm_output` is simply `None` while dark, exactly as before). `pipe()` /
    /// `pipe_mut()` reach the primary (first) output for the many single-output code
    /// paths; the render + vblank loops iterate `outputs` directly.
    pub outputs: Vec<OutputPipe>,
    pub drm_output_manager: Rc<RefCell<NativeDrmOutputManager>>,
    pub gpu_binding: Rc<RefCell<StateDRMBinding>>,
    pub libinput_context: Libinput,
    pub tap_subscriptions: TapSubscriptions,
    /// COMPOSITOR_RENDERER=vulkan: compose the scene with the VulkanRenderer and
    /// scan it out via the same DrmOutput (the GLES multigpu is still used for
    /// the per-frame iced/bevy/parallax GLES `prepare()`). Default false (GLES).
    pub vulkan_mode: bool,
    /// The native VulkanRenderer (built at wire time when vulkan_mode), bound to
    /// the primary render node.
    pub vulkan: Option<compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer>,
    /// Law-7 enablement, live: seeded from
    /// `compositor_kernel_graphic_preference_enable_safety::safety::get()` at wiring,
    /// runtime-updated through `device.interface` (the integration surface).
    pub safety: SafetyEnable,
    /// Raw DRM fd for the one-time HDR property commit (smithay's DrmCompositor
    /// doesn't expose colorspace / HDR metadata). Per-device (shared), so it stays
    /// on the context rather than the per-output [`OutputPipe`].
    pub drm_fd: smithay::backend::drm::DrmDeviceFd,
    /// The dark control-plane timer (`Timer` re-arming every ~100 ms) while the
    /// compositor has no output: pumps the important renderer-free drains so they
    /// progress with no rendering. `Some` only while dark — armed on the `WentDark`
    /// transition, removed on `Recovered`. See `wire.plugin` + `pump.dark`.
    pub dark_tick: Option<RegistrationToken>,
}

impl NativeRenderContext {
    /// The primary (first) output's pipe — the "current output" for the many
    /// single-output code paths (mode change, HDR signalling, session resume).
    /// Safe because `outputs` is always non-empty (see the field docs).
    pub fn pipe(&self) -> &OutputPipe {
        &self.outputs[0]
    }
    pub fn pipe_mut(&mut self) -> &mut OutputPipe {
        &mut self.outputs[0]
    }
}

