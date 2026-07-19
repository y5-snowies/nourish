//! Per-device settings applied on device-added. Designated home (seam):
//! enables touchpad tap-to-click. libinput synthesizes pointer buttons from
//! taps using its default button map (LRM): 1-finger tap -> BTN_LEFT (click),
//! 2-finger tap -> BTN_RIGHT, 3-finger tap -> BTN_MIDDLE.

use smithay::reexports::input::Device;

#[derive(Debug, Clone, Default)]
pub struct DeviceSettings {
    // Populated when per-device input configuration becomes a feature
    // (natural scroll, accel profile, ...).
}

pub fn on_device_added(device: &mut Device, _settings: &DeviceSettings) {
    // Only touchpads report a non-zero tap finger count; every other device
    // (keyboards, mice, ...) reports 0, so this is a no-op there. Enabling tap
    // makes libinput emit the pointer buttons the seat pipeline already
    // handles, so no downstream button plumbing is needed.
    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(true);
        trace!("touchpad tap-to-click enabled: {}", device.name());
    } else {
        trace!(
            "input device added (no tap settings applied): {}",
            device.name()
        );
    }
}
