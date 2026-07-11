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
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
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
    /// The card (DRM device) this pipe's CRTC lives on — the `dev_id` key into
    /// [`NativeRenderContext::devices`]. CRTC handles are only unique PER DEVICE, so
    /// vblank routing and manager selection key on `(device, crtc)`, never `crtc`
    /// alone. Single-device: every pipe carries the same `device`.
    pub device: DrmNode,
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
    /// OPTION-B + Vulkan: the render-node offscreen dmabuf this output's frame is
    /// composited into on the render GPU, before being handed to the scanout card
    /// (Vulkan blit → GLES-cross fallback). `(dmabuf, size)`; reallocated on resize.
    /// `None` unless `local_render` + `renderer: "vulkan"`.
    pub vk_offscreen: Option<(smithay::backend::allocator::dmabuf::Dmabuf, Mode)>,
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

/// One driven DRM device (card): the per-card KMS resources that used to be single
/// fields on [`NativeRenderContext`]. The compositing `GpuManager`
/// (`StateDRMBinding`) is NOT here — it is shared across all devices. Multiple
/// entries = monitors driven across multiple cards; the `OutputPipe`s on this card
/// carry `OutputPipe::device == node`.
pub struct DeviceRender {
    /// The card node — the `dev_id` key `OutputPipe::device` refers to.
    pub node: DrmNode,
    /// Raw DRM fd for this card's one-time HDR property commits (smithay's
    /// DrmCompositor doesn't expose colorspace / HDR metadata).
    pub drm_fd: DrmDeviceFd,
    /// This card's smithay output manager (owns its `DrmDevice`, allocator, CRTCs).
    pub drm_output_manager: Rc<RefCell<NativeDrmOutputManager>>,
    /// This card's calloop vblank source token (its own `DrmDeviceNotifier`), so a
    /// hotplugged card can be registered and a removed one torn down independently.
    pub vblank_token: Option<RegistrationToken>,
}

pub struct NativeRenderContext {
    pub display_handle: DisplayHandle,
    /// The lit outputs, one [`OutputPipe`] per driven CRTC. Invariant: NON-EMPTY —
    /// there is always at least one entry (in the single-output / dark eras its
    /// `drm_output` is simply `None` while dark, exactly as before). `pipe()` /
    /// `pipe_mut()` reach the primary (first) output for the many single-output code
    /// paths; the render + vblank loops iterate `outputs` directly.
    pub outputs: Vec<OutputPipe>,
    /// The driven DRM devices (cards), one [`DeviceRender`] per card. Invariant:
    /// NON-EMPTY; `devices[0]` is the primary card (the boot/anchor device). Each
    /// `OutputPipe` names its card via `OutputPipe::device`, resolved here through
    /// [`NativeRenderContext::manager_for`] / [`NativeRenderContext::drm_fd_for`].
    /// Single-device: exactly one entry, and every pipe's `device` equals it.
    pub devices: Vec<DeviceRender>,
    pub gpu_binding: Rc<RefCell<StateDRMBinding>>,
    pub libinput_context: Libinput,
    pub tap_subscriptions: TapSubscriptions,
    /// COMPOSITOR_RENDERER=vulkan: compose the scene with the VulkanRenderer and
    /// scan it out via the same DrmOutput (the GLES multigpu is still used for
    /// the per-frame iced/bevy/parallax GLES `prepare()`). Default false (GLES).
    pub vulkan_mode: bool,
    /// The native VulkanRenderer (built at wire time when vulkan_mode). Default: the
    /// scanout card. OPTION B (`local_render`): the RENDER node — it composites the
    /// scene into `OutputPipe::vk_offscreen`, which is then handed to the scanout card.
    pub vulkan: Option<compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer>,
    /// OPTION-B APPROACH B: a SECOND VulkanRenderer on the SCANOUT card. It imports the
    /// render-node offscreen and composites it into the scanout buffer (a GPU blit that
    /// does the untile). `None` unless `local_render` + Vulkan; when import fails
    /// (cross-vendor) the frame path falls back to the GLES-cross copy (approach A).
    pub vulkan_scanout: Option<compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer>,
    /// Law-7 enablement, live: seeded from
    /// `compositor_kernel_graphic_preference_enable_safety::safety::get()` at wiring,
    /// runtime-updated through `device.interface` (the integration surface).
    pub safety: SafetyEnable,
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

    /// The primary (anchor) card. Safe: `devices` is always non-empty.
    pub fn primary_device(&self) -> &DeviceRender {
        &self.devices[0]
    }

    /// The primary card's output manager (the "the device" for single-device paths).
    pub fn primary_manager(&self) -> &Rc<RefCell<NativeDrmOutputManager>> {
        &self.devices[0].drm_output_manager
    }

    /// The output manager for a specific card, falling back to the primary if the
    /// node is unknown (single-device: there is only the primary, so this always
    /// resolves to it).
    pub fn manager_for(&self, device: &DrmNode) -> &Rc<RefCell<NativeDrmOutputManager>> {
        self.devices
            .iter()
            .find(|d| d.node == *device)
            .map(|d| &d.drm_output_manager)
            .unwrap_or(&self.devices[0].drm_output_manager)
    }

    /// The raw DRM fd for a specific card (HDR property commits), falling back to the
    /// primary if the node is unknown.
    pub fn drm_fd_for(&self, device: &DrmNode) -> &DrmDeviceFd {
        self.devices
            .iter()
            .find(|d| d.node == *device)
            .map(|d| &d.drm_fd)
            .unwrap_or(&self.devices[0].drm_fd)
    }

    /// Mutable access to a specific card's [`DeviceRender`] (e.g. to store/clear its
    /// vblank token), by node.
    pub fn device_mut(&mut self, device: &DrmNode) -> Option<&mut DeviceRender> {
        self.devices.iter_mut().find(|d| d.node == *device)
    }
}

