//! The delegation host (Law 6): owns smithay's DrmOutputManager / DrmOutput
//! behind our typing. Sibling crates define their interfaces as if the pipe
//! were ours; the hosted objects carry the mechanism, and the (deferred)
//! de-delegation replaces them crate-by-crate.
//!
//! Failure policy: pipe bring-up must succeed (after the mode fallback chain
//! has had its say) — panic. Activate/reset keep local Results because their
//! one caller is the session-resume protocol, the designated self-recovering
//! path.

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, VrrSupport};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::{Bind, Renderer, Texture};
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::output::Output;
use smithay::reexports::drm::control::{connector, crtc, Mode as DrmMode};

/// The per-frame user data carried through queue_frame -> frame_submitted.
pub type FrameUserData = Option<OutputPresentationFeedback>;

/// Our names for the smithay pipe objects (the only place the full generic
/// signature is spelled out).
pub type NativeDrmOutput = DrmOutput<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    FrameUserData,
    DrmDeviceFd,
>;
pub type NativeDrmOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    FrameUserData,
    DrmDeviceFd,
>;

/// The color formats offered to the pipe. With `ten_bit`, 10-bit formats are
/// listed first and the 8-bit formats kept as a fallback — smithay negotiates
/// the first the plane supports, so a panel that can't scan out 10-bit falls
/// back to 8-bit rather than failing. `ten_bit` is requested both by HDR (PQ
/// needs the extra precision) and by plain deep-color SDR (COMPOSITOR_DEPTH=10):
/// the format choice is independent of the transfer function — 10-bit SDR scans
/// out the same sRGB values at finer quantization (less banding), no PQ.
///
/// **B-first leads at 10 bits, R-first at 8.** Not cosmetic: a KMS plane may
/// expose one channel order and not the other, and it is not the same order at
/// every depth. Measured on NVIDIA (595.80, RTX 4090), the primary plane offers
/// `AR15 AR24 XR15 XR24` — R-first — but at 10 bits offers only `AB30 XB30`, no
/// `AR30`/`XR30`. An R-first-only 10-bit ladder therefore misses every rung and
/// drops silently to 8-bit: smithay walks the ladder by design, so the
/// `NoSupportedPlaneFormat` warning per missed rung is indistinguishable from a
/// healthy probe, and `scanout_is_deep()` then reports 8-bit consistently to every
/// producer. A coherent 8-bit session with no artifact to notice — which is how
/// this went unseen. See `developer.tool.color/probe-change.MD`.
///
/// B-first is also the better-supported order across this tree: smithay's GLES
/// tables map only `Abgr2101010` (R-first 10-bit has no GL mapping at all), and
/// `negotiate.wgpu` omits `Argb2101010` for want of a wgpu `TextureFormat`. The
/// R-first pair stays as a fallback for hardware that exposes only it.
///
/// The 8-bit tail keeps `Argb8888` first — that path works today and reordering it
/// would change a format every producer is hardcoded to.

pub fn manager(
    drm: DrmDevice,
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    gbm: Option<GbmDevice<DrmDeviceFd>>,
    render_formats: FormatSet,
    ten_bit: bool,
) -> NativeDrmOutputManager {
    DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        gbm,
        compositor_kernel_graphic_format_catalog_base::catalog::scanout_ladder(ten_bit).into_iter(),
        render_formats,
    )
}

/// Bring the pipe online (ex wire.rs `new()` step 8). Returns Err so the mode
/// fallback chain (`commit.test`) can try the next candidate; the CHAIN
/// exhausting is the panic, at the assembly site.
#[allow(clippy::too_many_arguments)]
pub fn initialize<R, E>(
    manager: &mut NativeDrmOutputManager,
    pipe: crtc::Handle,
    mode: DrmMode,
    connectors: &[connector::Handle],
    output: &Output,
    renderer: &mut R,
) -> Result<NativeDrmOutput, String>
where
    // The vendored smithay's `initialize_output` requires these bounds on the
    // renderer; propagate them onto our delegation wrapper's `R`.
    R: Renderer + Bind<Dmabuf>,
    R::TextureId: Texture + 'static,
    R::Error: Send + Sync + 'static,
    E: RenderElement<R>,
{
    manager
        .lock()
        .initialize_output::<_, E>(
            pipe,
            mode,
            connectors,
            output,
            None,
            renderer,
            &DrmOutputRenderElements::default(),
        )
        .map_err(|e| format!("initialize_output failed: {e:?}"))
}

/// What `apply_vrr` found and did, for the caller to log and publish to stats.
pub struct VrrOutcome {
    pub supported: bool,
    pub enabled: bool,
    /// Human-readable trace of the probe + transition, for the log line.
    pub detail: String,
}

/// Apply the VRR / adaptive-sync request to this pipe's CRTC.
///
/// Must run for EVERY pipe, not just the one built at assembly. `use_vrr` writes
/// the surface's PENDING state, and `add_output` / `bring_up` construct a brand
/// new surface whose pending VRR defaults to false — so a pipe that never gets
/// this call silently scans out fixed-refresh no matter what the setting says.
/// That is why this lives here rather than inline in the assembly path: assembly,
/// the runtime builder and the fail-over rebuild all need the same call.
pub fn apply_vrr(output: &NativeDrmOutput, conn: connector::Handle, want: bool) -> VrrOutcome {
    let mut out = VrrOutcome { supported: false, enabled: false, detail: String::new() };
    output.with_compositor(|comp| {
        let probe = comp.vrr_supported(conn);
        out.supported = matches!(probe, Ok(VrrSupport::Supported | VrrSupport::RequiresModeset));
        let was = comp.vrr_enabled();
        out.detail = format!("probe={probe:?} supported={} want={want} was={was}", out.supported);
        if !want || !out.supported {
            out.enabled = false;
            return;
        }
        match comp.use_vrr(true) {
            Ok(()) => {
                out.enabled = comp.vrr_enabled();
                out.detail.push_str(&format!(" -> now={}", out.enabled));
            }
            Err(e) => {
                out.detail.push_str(&format!(" use_vrr FAILED: {e:?}"));
            }
        }
    });
    out
}

/// The pipe's negotiated scanout format + modifier set. Published so producers can
/// match the achieved depth instead of rendering 8-bit into a 10-bit pipeline.
pub fn format_info(output: &NativeDrmOutput) -> (Fourcc, smithay::backend::allocator::Modifier, Vec<String>) {
    let mut got = (
        compositor_kernel_graphic_format_catalog_base::catalog::FLOOR,
        compositor_kernel_graphic_format_rule_base::rule::UNKNOWN,
        Vec::new(),
    );
    output.with_compositor(|comp| {
        let all: Vec<String> = comp.modifiers().iter().map(|m| format!("{m:?}")).collect();
        got = (
            comp.format(),
            comp.modifiers().first().copied()
                .unwrap_or(compositor_kernel_graphic_format_rule_base::rule::UNKNOWN),
            all,
        );
    });
    got
}

/// Undo smithay's implicit-modifier fallback once the device is stable again.
///
/// When enabling an extra CRTC fails, `initialize_output` escalates: it lowers
/// bandwidth across ALL compositors and finally forces `DrmModifier::Invalid` on
/// every one of them. That is device-wide collateral — the pipes that were
/// working get dragged down with the one that failed — and it is never undone on
/// its own. With the Vulkan renderer it is fatal rather than merely slow: Vulkan
/// cannot create an image for an implicit-modifier buffer, so every `render_frame`
/// fails and the screen stays black.
///
/// smithay provides the recovery; nothing was calling it. Cheap when no pipe is on
/// implicit modifiers (it checks first and returns).
pub fn restore_modifiers<R, E>(
    manager: &mut NativeDrmOutputManager,
    renderer: &mut R,
) -> Result<(), String>
where
    R: Renderer + Bind<Dmabuf>,
    R::TextureId: Texture + 'static,
    R::Error: Send + Sync + 'static,
    E: RenderElement<R>,
{
    manager
        .lock()
        .try_to_restore_modifiers::<_, E>(renderer, &DrmOutputRenderElements::default())
        .map_err(|e| format!("try_to_restore_modifiers failed: {e:?}"))
}

/// Session-pause the whole device's pipes.
pub fn pause(manager: &mut NativeDrmOutputManager) {
    manager.pause();
}

/// Release the pipe's CRTC and planes while the session is STILL active.
///
/// smithay's surface `Drop` skips its disabling atomic commit once the device has
/// been deactivated (`AtomicDrmSurface::drop` early-returns on `!active`, assuming
/// the VT switch restores the old state). A pipe therefore leaves its CRTC
/// configured in the kernel whenever it is torn down after deactivation. Calling
/// this from the PAUSE path — before `pause()` flips the device inactive — is the
/// last moment a modeset can still be committed. `queue_frame` re-enables the
/// surface, so the resume path recovers it.
pub fn clear(output: &NativeDrmOutput) -> Result<(), String> {
    let mut result = Ok(());
    output.with_compositor(|compositor| {
        if let Err(err) = compositor.clear() {
            result = Err(format!("surface clear failed: {err:?}"));
        }
    });
    result
}

/// Session-activate; `force = true` performs the reclaiming modeset.
/// Result is for the resume protocol (self-recovering class).
pub fn activate(manager: &mut NativeDrmOutputManager, force: bool) -> Result<(), String> {
    manager
        .lock()
        .activate(force)
        .map_err(|e| format!("DRM activate failed: {e:?}"))
}

/// DPMS power the surface's connectors on/off without tearing down the pipe
/// (lid-close blank, idle). Recover an off display with `activate(force=true)`
/// + `reset`, exactly like the resume path. NOTE: any page-flip while off
/// re-powers the connector (legacy DPMS auto-on on commit), so the render loop
/// must be gated off in tandem.
pub fn set_dpms(output: &NativeDrmOutput, on: bool) -> Result<(), String> {
    let mut result = Ok(());
    output.with_compositor(|compositor| {
        if let Err(err) = compositor.surface().set_dpms(on) {
            result = Err(format!("surface set_dpms({on}) failed: {err:?}"));
        }
    });
    result
}

/// Reset the running surface state + buffers (resume steps 3-4).
/// Result is for the resume protocol (self-recovering class).
pub fn reset(output: &mut NativeDrmOutput) -> Result<(), String> {
    let mut result = Ok(());
    output.with_compositor(|compositor| {
        if let Err(err) = compositor.surface().reset_state() {
            result = Err(format!("surface reset_state failed: {err:?}"));
        }
    });
    output.reset_buffers();
    result
}
