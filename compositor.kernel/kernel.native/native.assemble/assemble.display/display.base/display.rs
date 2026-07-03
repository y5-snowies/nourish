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
    pub primary_gpu: DrmNode,
    pub device_path: PathBuf,
    /// Taken by `assemble.renderer` — the hosted DrmOutputManager owns the
    /// device, exactly as the original moved it into the manager.
    pub drm: Option<DrmDevice>,
    pub drm_notifier: DrmDeviceNotifier,
    pub drm_fd: DrmDeviceFd,
    pub gbm: GbmDevice<DrmDeviceFd>,
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

    // 2. Primary GPU: preference-aware selection over udev enumeration, with
    //    smithay's heuristic as the default (behavior-preserving when the
    //    preference is empty).
    let rank = compositor_kernel_graphic_preference_gpu_rank::rank::get();
    let candidates = compositor_kernel_udev_enumerate_gpu_base::gpu::all(&seat_name);
    let heuristic = compositor_kernel_udev_enumerate_gpu_base::gpu::primary(&seat_name);
    let selected_path = compositor_kernel_native_device_select_base::select::select_primary(
        &candidates,
        heuristic.as_ref(),
        &rank,
    );

    let primary_gpu = selected_path
        .as_deref()
        .and_then(compositor_kernel_drm_device_node_base::node::render_node)
        .or_else(|| {
            candidates
                .iter()
                .find_map(|p| smithay::backend::drm::DrmNode::from_path(p).ok())
        })
        .expect("No GPU!");

    // Record the gpu-topology decisions for the selected node (single-GPU
    // era: render and scanout are the same node — `route` proves it).
    let role = compositor_kernel_gpu_topology_role_base::role::assign(primary_gpu, Some(primary_gpu));
    let copy_route = compositor_kernel_gpu_topology_route_base::route::route(primary_gpu, primary_gpu);
    info!("gpu topology for selected node: role={role:?} copy_route={copy_route:?}");

    // 3. udev: find the device path whose dev_id matches the selected node.
    let primary_node = compositor_kernel_drm_device_node_base::node::primary_node(primary_gpu);
    let device_path = compositor_kernel_udev_enumerate_scan_base::scan::snapshot(&seat_name)
        .into_iter()
        .find(|(dev_id, _)| {
            compositor_kernel_drm_device_node_base::node::matches_dev(
                *dev_id,
                primary_gpu,
                primary_node,
            )
        })
        .map(|(_, path)| path)
        .expect("Could not find any usable DRM devices! Check seat configuration.");

    info!("Selected render node: {:?}", primary_gpu.dev_path());

    // 4. Open through the seat; wrap; DRM + GBM devices.
    let fd = compositor_kernel_seat_interface_open_base::open::open(&mut session, &device_path);
    let drm_fd = compositor_kernel_drm_device_open_base::open::wrap_fd(fd);
    let (drm, drm_notifier) = compositor_kernel_drm_device_open_base::open::open(drm_fd.clone());
    let gbm = compositor_kernel_drm_gbm_device_base::device::create(drm_fd.clone());

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
        device_path,
        drm: Some(drm),
        drm_notifier,
        drm_fd,
        gbm,
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
