//! Stable monitor identity for preferences + PhysicalProperties
//! (replacing the hardcoded "Native"/"Monitor"/"Unknown").

use compositor_kernel_drm_edid_parse_base::parse::ParsedEdid;

#[derive(Debug, Clone)]
pub struct MonitorIdentity {
    pub make: String,
    pub model: String,
    pub serial: String,
}

impl MonitorIdentity {
    /// The placeholder identity the original code hardcoded; used when no
    /// EDID is readable so behavior is unchanged on that path.
    pub fn unknown() -> Self {
        Self {
            make: "Native".into(),
            model: "Monitor".into(),
            serial: "Unknown".into(),
        }
    }

    /// The stable key preferences are matched against.
    pub fn key(&self) -> String {
        format!("{} {} {}", self.make, self.model, self.serial)
    }
}

/// Build the stable monitor identity from the parsed EDID. `fallback` (the unique
/// connector name, e.g. `DisplayPort-1`) is used for the serial when the EDID is
/// unreadable or carries no serial, so two identical / EDID-less monitors still get
/// DISTINCT keys (otherwise both collapse to "Native Monitor Unknown" and the
/// per-output render loop can't tell them apart). Priority: real EDID serial →
/// connector name; the key stays EDID-first, matching the settings editor + prefs.
pub fn identity(parsed: Option<&ParsedEdid>, fallback: &str) -> MonitorIdentity {
    match parsed {
        None => MonitorIdentity {
            make: "Native".into(),
            model: "Monitor".into(),
            serial: fallback.to_string(),
        },
        Some(p) => MonitorIdentity {
            make: p.manufacturer.clone(),
            model: p
                .display_name
                .clone()
                .unwrap_or_else(|| format!("{:04X}", p.product_code)),
            serial: if p.serial == 0 {
                fallback.to_string()
            } else {
                p.serial.to_string()
            },
        },
    }
}
