use crate::{ffi, AsRaw, FromRaw};

ffi_ref_struct! {
    /// A mode on a tablet pad is a virtual grouping of functionality, usually based on
    /// some visual feedback like LEDs on the pad.
    ///
    /// The set of buttons, rings and strips that share the same mode are a "mode
    /// group". Whenever the mode changes, all buttons, rings and strips within this
    /// mode group are affected. See
    /// [Tablet pad modes](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-pad-modes)
    /// for detail.
    ///
    /// Most tablets only have a single mode group, some tablets provide multiple mode
    /// groups through independent banks of LEDs (e.g. the Wacom Cintiq 24HD). libinput
    /// guarantees that at least one mode group is always available.
    struct TabletPadModeGroup, ffi::libinput_tablet_pad_mode_group, ffi::libinput_tablet_pad_mode_group_ref, ffi::libinput_tablet_pad_mode_group_unref
}

impl TabletPadModeGroup {
    /// The toggle button in a mode group is the button assigned to cycle to or
    /// directly assign a new mode when pressed.
    ///
    /// Not all devices have a toggle button and some devices may have more than
    /// one toggle button. For example, the Wacom Cintiq 24HD has six toggle
    /// buttons in two groups, each directly selecting one of the three modes per
    /// group.
    ///
    /// Devices without mode switching capabilities return `false` for every button.
    pub fn button_is_toggle(&self, button: u32) -> bool {
        unsafe {
            ffi::libinput_tablet_pad_mode_group_button_is_toggle(self.as_raw_mut(), button) != 0
        }
    }

    ffi_func!(
    /// The returned number is the same index as passed to `Device::tablet_pad_mode_group`.
    ///
    /// For tablets with only one mode this number is always 0.
    pub fn index, ffi::libinput_tablet_pad_mode_group_get_index, u32);
    ffi_func!(
    /// Return the current mode this mode group is in.
    ///
    /// Note that the returned mode is the mode valid as of completing the last
    /// `Libinput::dispatch`. The returned mode may thus be different than the mode
    /// returned by `TabletPadEvent::mode`.
    ///
    /// For example, if the mode was toggled three times between the call to
    /// `Libinput::dispatch`, this function returns the third mode but the events in the
    /// event queue will return the modes 1, 2 and 3, respectively.
    pub fn mode, ffi::libinput_tablet_pad_mode_group_get_mode, u32);
    ffi_func!(
    /// Query the mode group for the number of available modes.
    ///
    /// The number of modes is usually decided by the number of physical LEDs available on
    /// the device. Different mode groups may have a different number of modes. Use
    /// `TabletPadModeGroup::mode` to get the currently active mode.
    ///
    /// libinput guarantees that at least one mode is available. A device without mode
    /// switching capability has a single mode group and a single mode.
    pub fn number_of_modes, ffi::libinput_tablet_pad_mode_group_get_num_modes, u32);

    /// Devices without mode switching capabilities return `true` for every button.
    pub fn has_button(&self, button: u32) -> bool {
        unsafe { ffi::libinput_tablet_pad_mode_group_has_button(self.as_raw_mut(), button) != 0 }
    }

    /// Devices without mode switching capabilities return `true` for every ring.
    pub fn has_ring(&self, ring: u32) -> bool {
        unsafe { ffi::libinput_tablet_pad_mode_group_has_ring(self.as_raw_mut(), ring) != 0 }
    }

    /// Devices without mode switching capabilities return true for every dial.
    #[cfg(feature = "libinput_1_26")]
    pub fn has_dial(&self, dial: u32) -> bool {
        unsafe { ffi::libinput_tablet_pad_mode_group_has_dial(self.as_raw_mut(), dial) != 0 }
    }

    /// Devices without mode switching capabilities return `true` for every strip.
    pub fn has_strip(&self, strip: u32) -> bool {
        unsafe { ffi::libinput_tablet_pad_mode_group_has_strip(self.as_raw_mut(), strip) != 0 }
    }
}
