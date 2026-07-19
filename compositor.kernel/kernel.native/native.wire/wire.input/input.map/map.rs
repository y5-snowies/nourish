//! Correlate a libinput touch device to the DRM output it belongs to, so a touch
//! lands on its OWN display. Strongest signal first (à la GNOME `MetaInputMapper`);
//! `None` → caller discards rather than route to the wrong monitor:
//!   1. libinput `output_name` (explicit udev association, rarely set).
//!   2. Physical size — device mm ≈ output mm within 5%.
//!   3. EDID identity — device name contains the output's make/model/serial.
//!   4. Topology — USB touch → sole external output (or the internal panel if there
//!      is none); a non-USB touch → sole built-in panel. Ambiguity → settings.

use compositor_orchestration_core_state_base::state::output_key;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_driver_output_base::base::{
    TouchDeviceInfo, TouchDevicesSnapshot, TOUCH_DEVICES_SNAPSHOT_MUT,
};
// Backend `Device` trait (for `syspath()`); anon import avoids the struct clash.
use smithay::backend::input::Device as _;
use smithay::output::Output;
use smithay::reexports::input::Device;

/// Physical-size match tolerance (fraction) — `MetaInputMapper::MAX_SIZE_MATCH_DIFF`.
const MAX_SIZE_DIFF: f64 = 0.05;

/// A stable id for a touch device — its name plus the udev serial when present
/// (`"<name> <serial>"`), else just the name. Computed identically here and where
/// devices are enumerated for settings, so a user's claim matches the same device.
pub fn touch_device_id(device: &Device) -> String {
    let name = device.name();
    let serial = unsafe { device.udev_device() }
        .and_then(|d| d.property_value("ID_SERIAL_SHORT").map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty());
    match serial {
        Some(s) => format!("{name} {s}"),
        None => name.into_owned(),
    }
}

/// Republish the settings' connected-touch-device list from the seat. Call after
/// the seat's `touch_devices` change (device added/removed).
pub fn write_touch_snapshot(state: &mut Loop) {
    let devices: Vec<TouchDeviceInfo> = state
        .state
        .seat
        .touch_devices
        .iter()
        .map(|d| TouchDeviceInfo {
            id: touch_device_id(d),
            name: d.name().into_owned(),
            assigned_edid: None,
        })
        .collect();
    *state.inner.kernel.get_mut(&TOUCH_DEVICES_SNAPSHOT_MUT) = TouchDevicesSnapshot { devices };
}

pub fn touch_output(state: &Loop, device: &Device) -> Option<String> {
    let space = state.inner.space_state();

    // 0. Explicit user claim (settings Display tab) wins over every heuristic —
    // as long as the claimed monitor is currently driven.
    let id = touch_device_id(device);
    let claimed = state
        .inner
        .preference
        .outputs
        .iter()
        .find(|p| p.touch_device.as_deref() == Some(id.as_str()))
        .and_then(|p| p.identity.clone());
    if let Some(edid) = claimed {
        if space.state.outputs().any(|o| output_key(o) == edid) {
            return Some(edid);
        }
    }

    // 1. Explicit libinput output association.
    if let Some(name) = device.output_name() {
        if let Some(k) = space
            .state
            .outputs()
            .find(|o| o.name().as_str() == name.as_ref())
            .map(output_key)
        {
            return Some(k);
        }
    }

    // 2. Physical size (mm) within tolerance on both axes.
    if let Some((dw, dh)) = device.size().filter(|&(w, h)| w > 0.0 && h > 0.0) {
        if let Some(k) = space
            .state
            .outputs()
            .find(|o| {
                let s = o.physical_properties().size;
                s.w > 0
                    && s.h > 0
                    && (1.0 - s.w as f64 / dw).abs() <= MAX_SIZE_DIFF
                    && (1.0 - s.h as f64 / dh).abs() <= MAX_SIZE_DIFF
            })
            .map(output_key)
        {
            return Some(k);
        }
    }

    // 3. EDID identity embedded in the device name (make / model / serial).
    let dname = device.name().to_lowercase();
    let has = |s: &str| !s.is_empty() && dname.contains(s.to_lowercase().as_str());
    if let Some(k) = space
        .state
        .outputs()
        .find(|o| {
            let p = o.physical_properties();
            has(&p.make) || has(&p.model) || has(&p.serial_number)
        })
        .map(output_key)
    {
        return Some(k);
    }

    // 4. Topology: USB touch → the sole external output (or the internal panel when
    // there is no external); non-USB → the sole built-in. Ambiguous → None (settings).
    let external_touch = device
        .syspath()
        .and_then(|p| p.to_str().map(|s| s.contains("usb")))
        .unwrap_or(false);
    let externals: Vec<&Output> = space.state.outputs().filter(|o| !is_builtin(o)).collect();
    let builtins: Vec<&Output> = space.state.outputs().filter(|o| is_builtin(o)).collect();
    let pick = if external_touch {
        match externals.as_slice() {
            [only] => Some(*only),
            [] => match builtins.as_slice() {
                [only] => Some(*only),
                _ => None,
            },
            _ => None,
        }
    } else {
        match builtins.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    };
    pick.map(output_key)
}

/// A built-in panel: eDP / LVDS / DSI (name is the canonical DRM connector name).
fn is_builtin(output: &Output) -> bool {
    let n = output.name().to_ascii_lowercase();
    n.starts_with("edp") || n.starts_with("lvds") || n.starts_with("dsi")
}
