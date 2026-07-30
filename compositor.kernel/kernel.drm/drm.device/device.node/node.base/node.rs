//! DrmNode typing; render <-> primary node resolution.
//! (Ex wire.rs `new()` step 2, the node-mapping half.)

use smithay::backend::drm::{DrmNode, NodeType};
use std::path::Path;

/// Resolve a device path to its RENDER node, mirroring the original logic:
/// from_path -> node_with_type(Render), falling back to the node as-is.
pub fn render_node(path: &Path) -> Option<DrmNode> {
    let node = DrmNode::from_path(path).ok()?;
    node.node_with_type(NodeType::Render)
        .and_then(|r| r.ok())
        .or(Some(node))
}

/// The PRIMARY (card) node for a node, if resolvable.
pub fn primary_node(node: DrmNode) -> Option<DrmNode> {
    node.node_with_type(NodeType::Primary).and_then(|r| r.ok())
}

/// Whether a udev dev_t matches this node (or its primary sibling).
pub fn matches_dev(dev_id: u64, node: DrmNode, primary: Option<DrmNode>) -> bool {
    primary.map(|p| dev_id == p.dev_id()).unwrap_or(false) || dev_id == node.dev_id()
}

/// What a probe found out about a card node's display capability.
#[derive(Debug, Clone, Copy)]
pub struct Scanout {
    /// The kernel lists both connectors and CRTCs — this device CAN drive a display.
    pub capable: bool,
    /// At least one connector reads `Connected`. Strictly stronger than `capable`, and the
    /// tiebreaker when several devices are capable: a Raspberry Pi 5 exposes vc4's HDMI
    /// alongside RP1's DSI/DPI/VEC, so "has connectors" is true for more than one and
    /// picking the first is a coin toss. The monitor is on exactly one of them.
    pub connected: bool,
}

/// Probe `path`'s display capability. `None` means the question could not be ANSWERED and
/// the caller must not act on it — distinct from `Some(capable: false)`, which is proof.
///
/// `capable` is proven by the kernel listing both connectors and CRTCs. A device with
/// neither cannot scan out under any configuration — the render-only half of a split
/// render/display pair (Raspberry Pi: `v3d` has zero of both, `vc4` has both). Being
/// unplugged does NOT make a device incapable, which is why `connected` is reported
/// separately rather than folded in: an unplugged card is still display hardware, and
/// `connector.select` already reports the nothing-attached case on its own.
///
/// Read-only fd and read-only ioctls, so this needs neither DRM master nor the session and
/// is safe before the real libseat open. Failures are logged with their errno rather than
/// swallowed: `EBUSY` here means something else holds the device and is worth seeing, and
/// silently returning `None` for it once cost a debugging session.
pub fn probe_scanout(path: &Path) -> Option<Scanout> {
    use smithay::reexports::drm::control::{connector, Device as ControlDevice};
    use std::os::fd::{AsFd, BorrowedFd};

    struct Probe(std::fs::File);
    impl AsFd for Probe {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }
    }
    impl smithay::reexports::drm::Device for Probe {}
    impl ControlDevice for Probe {}

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            warn!("scanout probe: cannot open {path:?}: {e} — capability unknown");
            return None;
        }
    };
    let probe = Probe(file);
    let res = match probe.resource_handles() {
        Ok(r) => r,
        Err(e) => {
            trace!("scanout probe: {path:?} has no KMS resources ({e})");
            return None;
        }
    };
    let capable = !res.connectors().is_empty() && !res.crtcs().is_empty();
    let connected = res.connectors().iter().any(|c| {
        probe
            .get_connector(*c, false)
            .is_ok_and(|i| i.state() == connector::State::Connected)
    });
    Some(Scanout { capable, connected })
}
