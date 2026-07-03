//! Which connector to drive. Policy: honor the highest-priority monitor preference
//! whose monitor is currently connected — `profiles` are in priority order, so the
//! FIRST profile is the default output. With no profiles, no profile identities, or
//! no match among the connected monitors, the policy is the original first-connected
//! behavior (so this is behavior-neutral when the user has set no default).

use compositor_kernel_drm_edid_identity_base::identity;
use compositor_kernel_drm_edid_parse_base::parse;
use compositor_kernel_graphic_preference_output_profile::profile::OutputProfile;
use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::connector;

/// Pick the connector to drive. The first profile whose EDID identity
/// ("make model serial") matches a connected monitor wins; otherwise the first
/// connected connector is used. The EDID identity is the per-monitor key both the
/// in-compositor switch and the standalone settings-editor persist.
pub fn select(
    drm: &DrmDevice,
    connectors: Vec<connector::Info>,
    profiles: &[OutputProfile],
) -> Option<connector::Info> {
    let connected: Vec<connector::Info> = connectors
        .into_iter()
        .filter(|c| c.state() == connector::State::Connected)
        .collect();

    let chosen = profiles.iter().find_map(|p| {
        let want = p.identity.as_deref()?;
        connected.iter().position(|c| identity_key(drm, c) == want)
    });

    match chosen {
        Some(idx) => connected.into_iter().nth(idx),
        None => connected.into_iter().next(),
    }
}

/// The stable identity key ("make model serial") for a connector's monitor — the same
/// value both the in-compositor switch and the standalone settings editor key
/// preferences by. An unreadable EDID yields the unknown-monitor key, so it simply
/// never matches a real preference.
fn identity_key(drm: &DrmDevice, info: &connector::Info) -> String {
    let raw = parse::read(drm, info);
    let parsed = raw.as_ref().and_then(|r| parse::parse(r));
    identity::identity(
        parsed.as_ref(),
        &format!("{:?}-{}", info.interface(), info.interface_id()),
    )
    .key()
}
