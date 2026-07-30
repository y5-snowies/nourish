//! Standalone DRM probe: open each readable `/dev/dri/card*`, enumerate connected
//! connectors, and collect their advertised modes + a stable monitor identity key.
//! This runs OUTSIDE the compositor (e.g. during first install), so it can't read
//! the compositor's in-memory mode snapshot — it talks to DRM directly with the same
//! `drm` crate version smithay vendors. All ioctls used here (resource handles, get
//! connector with no force-probe, get properties, get property blob) are read-only
//! and need neither DRM master nor write access, so a plain read-only fd suffices.
//!
//! IDENTITY PARITY (critical): the identity key MUST match what the compositor
//! computes at runtime, or a saved preference silently won't apply. The block-0 EDID
//! parse and the key format below are a deliberate mirror of:
//!   - compositor.kernel/kernel.drm/drm.edid/edid.parse/parse.base/parse.rs  (block-0 fields)
//!   - compositor.kernel/kernel.drm/drm.edid/edid.identity/identity.base/identity.rs (key)
//! Those crates can't be reused here (they pull in smithay). Keep this in sync.

use drm::control::{connector, property, Device as ControlDevice, ModeTypeFlags};
use std::fs::{self, File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd};

/// A deduped advertised mode for one monitor.
pub struct ProbedMode {
    pub width: u16,
    pub height: u16,
    /// Refresh in millihertz: `mode.vrefresh() * 1000` — the SAME unit the compositor
    /// matches against (`m.vrefresh() * 1000 == refresh_mhz`).
    pub refresh_mhz: u32,
    pub preferred: bool,
}

impl ProbedMode {
    /// Whole-Hz refresh for display.
    pub fn refresh_hz(&self) -> u32 {
        self.refresh_mhz / 1000
    }
}

/// One connected monitor: its stable identity key (the preferences match key) plus a
/// human label and its advertised modes (preferred first, then largest area).
pub struct ProbedMonitor {
    pub identity_key: String,
    pub label: String,
    pub modes: Vec<ProbedMode>,
    /// The card node this monitor hangs off, e.g. `/dev/dri/card2` — i.e. a device
    /// that provably CAN scan out, which is what `scanout_node` wants. Kept because
    /// the card a connector lives on is not always the card holding `render_node`:
    /// where the render and display engines are separate DRM devices (Raspberry Pi:
    /// `v3d` owns `renderD128` with no CRTCs, `vc4` owns the connectors with no
    /// render node) they differ, and that is exactly when `scanout_node` must be set.
    pub node: String,
}

/// Minimal newtype so the opened DRM file satisfies the `drm` crate's marker traits.
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl drm::Device for Card {}
impl ControlDevice for Card {}

/// Probe every readable `/dev/dri/card*` and return one entry per connected monitor.
/// Returns an empty vec when nothing is openable (no permission, no DRM) — the caller
/// then offers a manual fallback.
pub fn probe() -> Vec<ProbedMonitor> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/dev/dri") else {
        return out;
    };
    let mut nodes: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.strip_prefix("card").is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit())))
        .collect();
    nodes.sort();

    for node in nodes {
        let path = format!("/dev/dri/{node}");
        let Ok(file) = OpenOptions::new().read(true).open(&path) else {
            continue;
        };
        let card = Card(file);
        let Ok(res) = card.resource_handles() else {
            continue;
        };
        for &handle in res.connectors() {
            let Ok(info) = card.get_connector(handle, false) else {
                continue;
            };
            if info.state() != connector::State::Connected {
                continue;
            }
            let modes = collect_modes(&info);
            if modes.is_empty() {
                continue;
            }
            let edid = read_edid(&card, &info).and_then(|raw| parse_identity(&raw));
            let identity_key = identity_key(edid.as_ref());
            let label = friendly_label(edid.as_ref(), &info);
            out.push(ProbedMonitor { identity_key, label, modes, node: path.clone() });
        }
    }
    out
}

/// The distinct card nodes that have at least one connected monitor, sorted — the
/// devices a `scanout_node` may legitimately name. Derived from [`probe`] rather than
/// from sysfs so "can scan out" means "the kernel listed a connected connector on it",
/// not a guess from path shape.
pub fn scanout_candidates(monitors: &[ProbedMonitor]) -> Vec<String> {
    let mut nodes: Vec<String> = monitors.iter().map(|m| m.node.clone()).collect();
    nodes.sort();
    nodes.dedup();
    nodes
}

/// Does `render_node` (e.g. `/dev/dri/renderD128`) live on the same physical device as
/// `card_node` (e.g. `/dev/dri/card2`)? Compares the sysfs `device` link both nodes
/// point at, which is the authoritative sibling test — their `dev_t`s differ, so
/// comparing paths or minor numbers would be wrong.
///
/// Pure `std::fs`: this only has to answer "same device", and canonicalizing
/// `/sys/class/drm/<node>/device` does that without opening the DRM device at all.
pub fn same_device(render_node: &str, card_node: &str) -> bool {
    let sysfs = |dev: &str| {
        let name = dev.rsplit('/').next()?;
        fs::canonicalize(format!("/sys/class/drm/{name}/device")).ok()
    };
    match (sysfs(render_node), sysfs(card_node)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Collect, dedup (by w×h×refresh) and order a connector's advertised modes:
/// preferred first, then by descending area, then descending refresh.
fn collect_modes(info: &connector::Info) -> Vec<ProbedMode> {
    let mut modes: Vec<ProbedMode> = Vec::new();
    for m in info.modes() {
        let (width, height) = m.size();
        let refresh_mhz = m.vrefresh() * 1000;
        let preferred = m.mode_type().contains(ModeTypeFlags::PREFERRED);
        if let Some(existing) = modes
            .iter_mut()
            .find(|e| e.width == width && e.height == height && e.refresh_mhz == refresh_mhz)
        {
            existing.preferred |= preferred;
        } else {
            modes.push(ProbedMode { width, height, refresh_mhz, preferred });
        }
    }
    modes.sort_by(|a, b| {
        b.preferred
            .cmp(&a.preferred)
            .then((b.width as u32 * b.height as u32).cmp(&(a.width as u32 * a.height as u32)))
            .then(b.refresh_mhz.cmp(&a.refresh_mhz))
    });
    modes
}

/// Read the connector's EDID property blob, if present. Mirrors the compositor's
/// `parse::read` property walk, using the plain `drm` crate.
fn read_edid(card: &Card, info: &connector::Info) -> Option<Vec<u8>> {
    let props = card.get_properties(info.handle()).ok()?;
    for (prop, value) in props.iter() {
        let Ok(prop_info) = card.get_property(*prop) else {
            continue;
        };
        if prop_info.name().to_str() != Ok("EDID") {
            continue;
        }
        if let property::Value::Blob(blob_id) = prop_info.value_type().convert_value(*value) {
            if blob_id == 0 {
                return None;
            }
            if let Ok(blob) = card.get_property_blob(blob_id) {
                return Some(blob);
            }
        }
    }
    None
}

/// The block-0 EDID identity fields (mirror of `parse.rs::ParsedEdid` + `parse`).
struct Identity {
    manufacturer: String,
    product_code: u16,
    serial: u32,
    display_name: Option<String>,
}

/// Parse the EDID identity fields from a raw blob (mirror of `parse.rs::parse`).
fn parse_identity(d: &[u8]) -> Option<Identity> {
    const MAGIC: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if d.len() < 128 || d[0..8] != MAGIC {
        return None;
    }
    let m = u16::from_be_bytes([d[8], d[9]]);
    let letter = |v: u16| ((v & 0x1F) as u8 + b'A' - 1) as char;
    let manufacturer: String = [letter(m >> 10), letter(m >> 5), letter(m)].iter().collect();
    let product_code = u16::from_le_bytes([d[10], d[11]]);
    let serial = u32::from_le_bytes([d[12], d[13], d[14], d[15]]);

    let mut display_name = None;
    for desc in 0..4 {
        let off = 54 + desc * 18;
        if d[off] == 0 && d[off + 1] == 0 && d[off + 3] == 0xFC {
            let name: String = d[off + 5..off + 18]
                .iter()
                .take_while(|b| **b != 0x0A)
                .map(|b| *b as char)
                .collect();
            display_name = Some(name.trim().to_string());
        }
    }
    Some(Identity { manufacturer, product_code, serial, display_name })
}

/// The stable preference key (mirror of `identity.rs::identity` + `MonitorIdentity::key`).
/// `None` (no/invalid EDID) yields the compositor's unknown fallback key.
fn identity_key(id: Option<&Identity>) -> String {
    match id {
        None => "Native Monitor Unknown".to_string(),
        Some(p) => {
            let make = &p.manufacturer;
            let model = p
                .display_name
                .clone()
                .unwrap_or_else(|| format!("{:04X}", p.product_code));
            let serial = if p.serial == 0 { "Unknown".to_string() } else { p.serial.to_string() };
            format!("{make} {model} {serial}")
        }
    }
}

/// A human label for the picker: EDID make+model when available, else the connector
/// interface name (e.g. "DisplayPort-1"). Not persisted — display only.
fn friendly_label(id: Option<&Identity>, info: &connector::Info) -> String {
    let connector = format!("{}-{}", info.interface().as_str(), info.interface_id());
    match id {
        Some(p) => {
            let model = p
                .display_name
                .clone()
                .unwrap_or_else(|| format!("{:04X}", p.product_code));
            format!("{} {} ({connector})", p.manufacturer, model)
        }
        None => connector,
    }
}
