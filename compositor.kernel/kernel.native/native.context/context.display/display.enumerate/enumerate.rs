//! Enumerate every CONNECTED connector on the driven DRM device into the
//! primitive `OutputsSnapshot` the settings Display panel reads — so the UI can
//! offer a preferred-monitor picker and list each monitor's advertised modes,
//! including connected-but-inactive ones. Probing connectors (force-probe, like
//! every hotplug) reads their modes/EDID without modesetting the active pipe.

use compositor_kernel_drm_edid_identity_base::identity;
use compositor_kernel_drm_edid_parse_base::parse;
use compositor_kernel_graphic_preference_output_profile::profile::{self, ModeRequest};
use compositor_orchestration_driver_output_base::base::{DisplayInfo, ModeInfo, OutputsSnapshot};
use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::{connector, Mode as DrmMode};

fn to_info(m: &DrmMode) -> ModeInfo {
    ModeInfo { width: m.size().0, height: m.size().1, refresh_mhz: m.vrefresh() * 1000 }
}

/// The mode saved in preferences for the monitor keyed by `edid_key` (its per-output
/// profile's advertised mode), if set — so the picker can default an inactive monitor
/// to its saved mode rather than the recommended one.
fn preferred_mode(edid_key: &str) -> Option<ModeInfo> {
    profile::get()
        .iter()
        .find(|p| p.identity.as_deref() == Some(edid_key))
        .and_then(|p| p.mode.as_ref())
        .and_then(|m| match m {
            ModeRequest::Advertised { width, height, refresh_mhz } => {
                Some(ModeInfo { width: *width, height: *height, refresh_mhz: *refresh_mhz })
            }
            _ => None,
        })
}

/// One `DisplayInfo` per CONNECTED connector. `active` marks the connector treated
/// as primary; `lit` gives the current mode of EVERY driven pipe (by connector), so
/// each connected monitor reports its own `current` (multi-output) — not just the
/// primary. A connector absent from `lit` (connected but not driven) has no current.
pub fn enumerate(
    drm: &DrmDevice,
    active: connector::Handle,
    lit: &[(connector::Handle, ModeInfo)],
) -> OutputsSnapshot {
    let res = compositor_kernel_drm_connector_scan_base::scan::resources(drm);
    let infos = compositor_kernel_drm_connector_scan_base::scan::connectors(drm, &res);
    let mut displays = Vec::new();
    for info in &infos {
        if info.state() != connector::State::Connected {
            continue;
        }
        // The stable per-monitor key is the EDID identity ("make model serial",
        // incl. the unit's serial so two identical monitors differ) — the SAME key
        // the standalone settings-editor persists, so a preference set in either
        // place matches. The connector name is only a friendlier label suffix.
        let conn_name = format!("{:?}-{}", info.interface(), info.interface_id());
        let raw = parse::read(drm, info);
        let parsed = raw.as_ref().and_then(parse::parse);
        let id = identity::identity(parsed.as_ref(), &conn_name);
        let edid_key = id.key();
        let label = if parsed.is_some() {
            format!("{} {} ({conn_name})", id.make, id.model)
        } else {
            conn_name
        };
        let is_active = info.handle() == active;
        let available: Vec<ModeInfo> = info.modes().iter().map(to_info).collect();
        // Each DRIVEN connector reports its own live mode; a connected-but-undriven
        // one has no "current" and the UI defaults a selection from `available`.
        let current = lit.iter().find(|(h, _)| *h == info.handle()).map(|(_, m)| *m);
        let preferred = preferred_mode(&edid_key);
        // User enable/disable (the settings "Inactive" toggle); profile-less → enabled.
        let enabled = compositor_kernel_graphic_preference_output_profile::profile::active(&edid_key);
        displays.push(DisplayInfo {
            edid_key,
            name: label,
            connected: true,
            active: is_active,
            enabled,
            current,
            preferred,
            available,
        });
    }
    OutputsSnapshot { displays }
}
