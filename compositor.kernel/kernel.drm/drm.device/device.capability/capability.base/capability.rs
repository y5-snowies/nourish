//! Functional KMS capability probe for a DRM card fd. This does NOT trust sysfs
//! CRTC counts or the `DRIVER_MODESET` flag — it exercises the exact ioctl the
//! scanout path depends on (`drmModeGetResources`), which is precisely what a
//! render-only node (e.g. Tegra `nvgpu`) fails with EOPNOTSUPP. A card is
//! scanout-capable only if modeset works AND it has ≥1 CRTC and ≥1 connector.
//! The fd is opened through the seat by the caller; the resource query needs no
//! DRM master, so probing is side-effect-free.

use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::drm::control::{connector, Device as ControlDevice};
use smithay::reexports::drm::Device as BasicDevice;

/// The result of probing one card. `driver` is the kernel driver name (for
/// diagnostics / the verbose panic); `None` if it could not be read.
#[derive(Debug, Clone)]
pub struct KmsCapability {
    /// `drmModeGetResources` succeeded (the modeset API is present).
    pub modeset: bool,
    pub crtcs: usize,
    pub connectors: usize,
    /// Connectors currently reporting `Connected` — the #2 tiebreak (a card
    /// actually driving a display) when several cards are KMS-capable.
    pub connected: usize,
    pub driver: Option<String>,
}

impl KmsCapability {
    /// The gate: can this card drive a display? Modeset + at least one CRTC and
    /// one connector. This is the ONLY predicate the scanout selection trusts.
    pub fn is_scanout_capable(&self) -> bool {
        self.modeset && self.crtcs > 0 && self.connectors > 0
    }
}

/// Probe a seat-opened card fd for KMS/scanout capability.
pub fn probe(device: &DrmDeviceFd) -> KmsCapability {
    let driver = device
        .get_driver()
        .ok()
        .map(|d| d.name().to_string_lossy().into_owned());

    match device.resource_handles() {
        Ok(res) => {
            let crtcs = res.crtcs().len();
            let connectors = res.connectors().len();
            // Light probe (`force = false`): don't trigger a full detect on cards
            // we may not even use — connected-count is only a tiebreak.
            let connected = res
                .connectors()
                .iter()
                .filter(|c| {
                    device
                        .get_connector(**c, false)
                        .map(|i| i.state() == connector::State::Connected)
                        .unwrap_or(false)
                })
                .count();
            KmsCapability { modeset: true, crtcs, connectors, connected, driver }
        }
        // EOPNOTSUPP here = render-only node (no KMS). Not an error to log: the
        // caller decides whether an un-capable card is fatal (see selection).
        Err(_) => KmsCapability { modeset: false, crtcs: 0, connectors: 0, connected: 0, driver },
    }
}
