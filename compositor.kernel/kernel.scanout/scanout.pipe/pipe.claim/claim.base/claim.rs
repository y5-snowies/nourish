//! Pipe (CRTC) claim policy: current-encoder walk, then a `possible_crtcs`-routable
//! CRTC, then crtcs[0].
//! (Ex wire.rs `new()` step 6, the CRTC half.)

use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::{connector, crtc, Device, ResourceHandles};

/// A CRTC for `connector`. The CRTC already driving it wins (nothing to re-route);
/// otherwise walk the connector's encoders for a CRTC it can actually be routed to.
///
/// The routability walk is why `crtcs[0]` is a LAST resort rather than the only
/// fallback: on a device exposing several CRTCs where only some reach this
/// connector — vc4 on a Raspberry Pi carries HDMI0/HDMI1 plus writeback and
/// DSI/DPI pipes — `crtcs[0]` is usually not one of the connector's. Handing an
/// unroutable CRTC to the modeset makes EVERY candidate mode, format and modifier
/// fail the atomic test identically, which reads like a format-negotiation problem
/// and is not one. `claim.free` has always walked `possible_crtcs`; this is the
/// same walk, minus the busy-set exclusion (initial assembly has no other pipe).
pub fn claim(
    drm: &DrmDevice,
    connector: &connector::Info,
    res: &ResourceHandles,
) -> Option<crtc::Handle> {
    connector
        .current_encoder()
        .and_then(|enc_handle| drm.get_encoder(enc_handle).ok())
        .and_then(|encoder| encoder.crtc())
        .or_else(|| routable(drm, connector, res))
        .or_else(|| res.crtcs().first().copied())
}

/// The first CRTC any of `connector`'s encoders can be routed to.
fn routable(
    drm: &DrmDevice,
    connector: &connector::Info,
    res: &ResourceHandles,
) -> Option<crtc::Handle> {
    for enc in connector.encoders() {
        let Ok(info) = drm.get_encoder(*enc) else {
            continue;
        };
        if let Some(c) = res.filter_crtcs(info.possible_crtcs()).into_iter().next() {
            return Some(c);
        }
    }
    None
}
