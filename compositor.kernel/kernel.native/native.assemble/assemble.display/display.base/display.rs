//! Display-side assembly: session -> enumeration -> selection -> device open
//! -> gbm -> connector -> pipe -> mode (+ fallback chain, + gated synthesis)
//! -> EDID identity -> Output. (Ex wire.rs `new()` steps 1-7, recomposed.)
//! Failure policy: any step failing here means no display — panic, exactly
//! as the original's unwraps did.

use compositor_kernel_drm_connector_diff_base::diff::ConnectorSnapshot;
use compositor_kernel_drm_edid_identity_base::identity::MonitorIdentity;
use compositor_kernel_graphic_preference_output_profile::profile::ModeRequest;
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmNode};
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::backend::session::Session;
use smithay::output::{Mode, Output};
use smithay::reexports::drm::control::{connector, crtc, Mode as DrmMode};
use std::path::PathBuf;

/// Everything the display half of assembly produced. Field-for-field this is
/// the display side of the old `state::Udev` struct plus the intermediate
/// values the renderer half consumes.
pub struct DisplayAssembly {
    pub session: LibSeatSession,
    pub session_notifier: LibSeatSessionNotifier,
    pub seat_name: String,
    /// The RENDER node (wgpu pin + compositing renderer). Equals the scanout
    /// card's node on a normal desktop; differs on a split (PRIME) system.
    pub primary_gpu: DrmNode,
    /// Render vs scanout relationship. `None` = same device (desktop, direct
    /// render-into-scanout). `DmabufCopy` = split; the composited frame must
    /// cross from `render` to `scanout` (import if the dmabuf is scannable on the
    /// scanout card, else a blit). Consumed by `assemble.renderer`/`render.execute`.
    pub route: compositor_kernel_gpu_topology_route_base::route::CopyRoute,
    pub device_path: PathBuf,
    /// Taken by `assemble.renderer` — the hosted DrmOutputManager owns the
    /// device, exactly as the original moved it into the manager.
    pub drm: Option<DrmDevice>,
    pub drm_notifier: DrmDeviceNotifier,
    pub drm_fd: DrmDeviceFd,
    pub gbm: GbmDevice<DrmDeviceFd>,
    /// OPTION B (`local_render`): a GBM device on the RENDER node itself, opened
    /// only when the primary carries `local_render` on a split (`route` =
    /// `DmabufCopy`). `Some` ⇒ `assemble.renderer` composites on the render node
    /// and imports to the scanout card; `None` (default) ⇒ today's
    /// composite-directly-on-scanout path, byte-identical.
    pub render_gbm: Option<GbmDevice<DrmDeviceFd>>,
    pub connector: connector::Info,
    pub pipe: crtc::Handle,
    pub drm_mode: DrmMode,
    /// The validating-modeset fallback chain (selected mode first); consumed
    /// by `assemble.renderer` around pipe bring-up.
    pub mode_chain: Vec<DrmMode>,
    /// The full connector state at assembly — the hotplug diff baseline
    /// (`context.topology` stores it; `plugin.route` compares against it).
    pub initial_snapshot: ConnectorSnapshot,
    pub mode: Mode,
    pub output: Output,
    pub identity: MonitorIdentity,
    /// HDR / colorimetry capabilities parsed from EDID (defaults to "no HDR"
    /// when the EDID is unreadable or SDR-only). Consumed by the M5 HDR path.
    pub hdr: compositor_kernel_drm_edid_parse_base::parse::HdrInfo,
}

pub fn assemble() -> DisplayAssembly {
    info!("Init native backend (assemble.display)");

    // 1. Session via libseat.
    let (mut session, session_notifier) =
        compositor_kernel_seat_session_factory_base::factory::create();
    let seat_name = session.seat();

    // 2-3. Render + scanout (PRIME-aware) resolution. The RENDER node is anchored
    //    to the configured `render_node` (so smithay's device agrees with the wgpu
    //    pin); the SCANOUT card is resolved by an explicit KMS capability probe.
    //    On a normal desktop the render card is itself KMS-capable, so it IS the
    //    scanout card (`route` None) — byte-identical to the pre-split path. Only a
    //    render-only GPU (e.g. Tegra `nvgpu`) diverges into `DmabufCopy`; a selected
    //    card with no KMS is a verbose panic inside `resolve`, never a silent fallback.
    let heuristic = compositor_kernel_udev_enumerate_gpu_base::gpu::primary(&seat_name);
    let render_pref = compositor_kernel_graphic_preference_gpu_rank::rank::render_node();
    let scanout_pref = compositor_kernel_graphic_preference_gpu_rank::rank::scanout_node();
    let cards = compositor_kernel_udev_enumerate_scan_base::scan::snapshot(&seat_name);
    let render_fallback = compositor_kernel_graphic_preference_gpu_rank::rank::render_fallback();
    let scan_fallback = compositor_kernel_graphic_preference_gpu_rank::rank::scan_fallback();
    let resolved = compositor_kernel_native_device_select_scanout::select::resolve(
        &mut session,
        &cards,
        render_pref.as_deref(),
        heuristic.as_deref(),
        scanout_pref.as_deref(),
        render_fallback,
        scan_fallback,
    );
    let primary_gpu = resolved.render;
    let device_path = resolved.scanout_path.clone();
    let copy_route = resolved.route;

    // Topology roles are per-node; the scanout card is what smithay drives here.
    let role =
        compositor_kernel_gpu_topology_role_base::role::assign(resolved.scanout, Some(resolved.scanout));
    info!(
        "gpu topology: role={role:?} route={copy_route:?} render={:?} scanout={:?}",
        primary_gpu.dev_path(),
        resolved.scanout.dev_path()
    );
    info!("Selected scanout card: {device_path:?} (render node {:?})", primary_gpu.dev_path());
    if matches!(copy_route, compositor_kernel_gpu_topology_route_base::route::CopyRoute::DmabufCopy { .. }) {
        warn!(
            "SPLIT render/scanout active (render {:?} != scanout {:?}). Compositing into the \
             scanout card's GBM buffer via cross-device dmabuf import. On unified-memory SoCs \
             (e.g. Tegra/Orin) this import should succeed with no copy; if the display stays \
             black or import fails in the renderer, the explicit blit path (Stage 4 fallback) \
             is required. See document/GPU_TOPOLOGY.md.",
            primary_gpu.dev_path(),
            resolved.scanout.dev_path()
        );
    }

    // 4. DRM + GBM on the already-opened, KMS-probed scanout fd (no second open on
    //    the desktop path). GBM lives on the scanout card; the split render-side
    //    allocator + copy is wired by the DmabufCopy route (see `copy_route`).
    let drm_fd = resolved.scanout_fd.clone();
    let (drm, drm_notifier) = compositor_kernel_drm_device_open_base::open::open(drm_fd.clone());
    let gbm = compositor_kernel_drm_gbm_device_base::device::create(drm_fd.clone());

    // OPTION B (`local_render` on the primary, split system): open a GBM on the
    // RENDER node so `assemble.renderer` can composite there and cross to scanout.
    // Default (no token / non-split) → None → composite-on-scanout, unchanged.
    let render_gbm = {
        use compositor_kernel_gpu_topology_route_base::route::CopyRoute;
        let is_split = matches!(copy_route, CopyRoute::DmabufCopy { .. });
        if compositor_developer_environment_config_router::router::composite_on_render_node() && is_split
        {
            // STRICT: a cross-device compositing route needs an explicit path token
            // (`zero_copy`/`untile_blit`); `plan::decide` returns Err otherwise. The
            // default option-A path composites on scanout with no copy, so this gate
            // never fires there — today's tokenless splits are untouched.
            if let Err(msg) = compositor_kernel_gpu_topology_plan_base::plan::decide(
                &copy_route,
                compositor_developer_environment_config_router::router::primary_mode(),
            ) {
                abort!("option B (local_render): {msg}");
            }
            let render_path = compositor_developer_environment_config_router::router::primary_render();
            // A RENDER node (`renderD*`) is NOT seat-managed — `seat_open` returns
            // ENODEV. It needs no DRM master either; open it directly (RDWR) for GBM
            // allocation. (Only the KMS SCANOUT card goes through libseat/master.)
            let render_file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&render_path)
                .unwrap_or_else(|e| {
                    abort!("option B (local_render): cannot open render node {render_path:?}: {e}")
                });
            let render_fd =
                compositor_kernel_drm_device_open_base::open::wrap_fd(render_file.into());
            info!("option B (local_render): opened render node {render_path:?} for its own compositing GBM");
            Some(compositor_kernel_drm_gbm_device_base::device::create(render_fd))
        } else {
            None
        }
    };

    // 5. Connector: scan, select (preference default-output identity, else first
    //    connected). `profiles` are priority-ordered; the first is the default.
    let res = compositor_kernel_drm_connector_scan_base::scan::resources(&drm);
    let connectors = compositor_kernel_drm_connector_scan_base::scan::connectors(&drm, &res);
    let profiles = compositor_kernel_graphic_preference_output_profile::profile::get();
    let initial_snapshot = ConnectorSnapshot::take(&connectors);
    let connector =
        compositor_kernel_drm_connector_select_base::select::select(&drm, connectors, &profiles)
            .expect("No connected monitor found");
    let kind = compositor_kernel_drm_connector_kind_base::kind::classify(&connector);
    info!("selected connector classified: {kind:?}");

    // 6. Pipe claim.
    let pipe = compositor_kernel_scanout_pipe_claim_base::claim::claim(&drm, &connector, &res)
        .expect("no CRTC available");
    let _assignment =
        compositor_kernel_scanout_pipe_assign_base::assign::assign(connector.handle(), pipe);

    // 7. Mode: profile request (advertised narrows; synthesis is the gated
    //    arm) -> default policy -> diagnostics -> fallback chain.
    let drm_mode = resolve_mode(&connector, profiles.first());
    compositor_kernel_drm_mode_select_base::select::log_selected(&drm_mode);
    compositor_kernel_drm_mode_enumerate_base::enumerate::dump(&connector);
    let mode_chain =
        compositor_kernel_scanout_commit_test_base::test::fallback_chain(&connector, drm_mode);

    // 8. EDID identity (placeholder identity when unreadable — behavior-
    //    preserving) + orientation + Output.
    let raw = compositor_kernel_drm_edid_parse_base::parse::read(&drm, &connector);
    let parsed = raw
        .as_ref()
        .and_then(compositor_kernel_drm_edid_parse_base::parse::parse);
    let identity = compositor_kernel_drm_edid_identity_base::identity::identity(
        parsed.as_ref(),
        &format!("{:?}-{}", connector.interface(), connector.interface_id()),
    );
    let hdr = raw
        .as_ref()
        .map(compositor_kernel_drm_edid_parse_base::parse::parse_hdr)
        .unwrap_or_default();
    info!(
        "display HDR caps: pq={} hlg={} bt2020_rgb={} max_lum={:?}",
        hdr.hdr.eotf_pq, hdr.hdr.eotf_hlg, hdr.colorimetry.bt2020_rgb, hdr.hdr.max_luminance
    );
    let orientation =
        compositor_kernel_drm_connector_kind_base::kind::panel_orientation(&drm, &connector);

    let output = compositor_kernel_drm_output_physical_base::physical::create(&connector, &identity);
    let mode = Mode::from(drm_mode);
    let position =
        compositor_kernel_graphic_preference_layout_output::output::position_for(Some(&identity.key()), 0);
    compositor_kernel_drm_output_physical_base::physical::apply_initial_state(
        &output,
        mode,
        orientation,
        (position.0, position.1),
    );

    DisplayAssembly {
        session,
        session_notifier,
        seat_name,
        primary_gpu,
        route: copy_route,
        device_path,
        drm: Some(drm),
        drm_notifier,
        drm_fd,
        gbm,
        render_gbm,
        connector,
        pipe,
        drm_mode,
        mode_chain,
        initial_snapshot,
        mode,
        output,
        identity,
        hdr,
    }
}

/// Resolve the mode for a connector against an (optional) profile request.
/// Advertised requests narrow the advertised list (`drm.mode/mode.select`);
/// synthesis requests are the Law-7 double gate: the `mode-synthesize`
/// feature compiles the arm in, `SafetyEnable::mode_synthesize` authorizes
/// it, and a request without both is a configuration error — panic.
fn resolve_mode(
    connector: &connector::Info,
    profile: Option<&compositor_kernel_graphic_preference_output_profile::profile::OutputProfile>,
) -> DrmMode {
    use compositor_kernel_graphic_preference_output_profile::profile::OutputProfile;
    match profile.and_then(|p| p.mode.as_ref()) {
        Some(ModeRequest::Cvt { .. }) | Some(ModeRequest::Modeline(_)) => {
            synthesize_mode(profile.unwrap())
        }
        Some(ModeRequest::Advertised { .. }) => {
            compositor_kernel_drm_mode_select_base::select::select(connector, profile)
                .expect("connector advertises no modes")
        }
        // No per-output mode: try the hand-set default mode (advertised match),
        // else fall through to the default selection policy. An unmatched
        // advertised request inside mode.select falls back to default policy too.
        None => {
            let dm = compositor_kernel_graphic_preference_output_profile::profile::default_mode()
                .map(|mode| OutputProfile { identity: None, mode: Some(mode), active: true });
            compositor_kernel_drm_mode_select_base::select::select(connector, dm.as_ref())
                .expect("connector advertises no modes")
        }
    }
}

#[cfg(feature = "mode-synthesize")]
fn synthesize_mode(
    profile: &compositor_kernel_graphic_preference_output_profile::profile::OutputProfile,
) -> DrmMode {
    use compositor_kernel_drm_mode_synthesize_base::synthesize;
    assert!(
        compositor_kernel_graphic_preference_enable_safety::safety::get().mode_synthesize,
        "mode synthesis requested by a profile but SafetyEnable::mode_synthesize is off"
    );
    let timing = match profile.mode.as_ref().unwrap() {
        ModeRequest::Cvt { width, height, refresh } => {
            synthesize::cvt_rb(*width, *height, *refresh)
        }
        ModeRequest::Modeline(s) => synthesize::parse_modeline(s)
            .unwrap_or_else(|e| abort!("malformed modeline in output profile: {e}")),
        ModeRequest::Advertised { .. } => unreachable!("advertised handled by mode.select"),
    };
    let mode = synthesize::to_drm_mode(timing);
    warn!(
        "mode-synthesize active: driving a non-advertised mode {}x{}@{}",
        mode.size().0,
        mode.size().1,
        mode.vrefresh()
    );
    mode
}

#[cfg(not(feature = "mode-synthesize"))]
fn synthesize_mode(
    _profile: &compositor_kernel_graphic_preference_output_profile::profile::OutputProfile,
) -> DrmMode {
    abort!(
        "an output profile requests mode synthesis but the backend was built without the \
         `mode-synthesize` feature"
    );
}
