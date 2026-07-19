use crate::{ffi, AsRaw, FromRaw, Libinput};
use std::{borrow::Cow, ffi::CStr};

ffi_ref_struct!(
/// A seat has two identifiers, the physical name and the logical name.
///
/// A device is always assigned to exactly one seat. It may change to a
/// different logical seat but it cannot change physical seats. See
/// [Seats](https://wayland.freedesktop.org/libinput/doc/latest/seats.html)
/// for details.
struct Seat, ffi::libinput_seat, ffi::libinput_seat_ref, ffi::libinput_seat_unref);

impl Seat {
    /// Get the libinput context from the seat.
    pub fn context(&self) -> Libinput {
        self.context.clone()
    }

    /// Return the physical name of the seat.
    ///
    /// For libinput contexts created from udev, this is always the
    /// same value as passed into `udev_assign_seat` and all
    /// seats from that context will have the same physical name.
    ///
    /// The physical name of the seat is one that is usually set by the
    /// system or lower levels of the stack. In most cases, this is the
    /// base filter for devices - devices assigned to seats outside the
    /// current seat will not be available to the caller.
    pub fn physical_name(&self) -> Cow<'_, str> {
        unsafe {
            CStr::from_ptr(ffi::libinput_seat_get_physical_name(self.as_raw_mut()))
                .to_string_lossy()
        }
    }

    /// Return the logical name of the seat.
    ///
    /// This is an identifier to group sets of devices within the
    /// compositor.
    pub fn logical_name(&self) -> Cow<'_, str> {
        unsafe {
            CStr::from_ptr(ffi::libinput_seat_get_logical_name(self.as_raw_mut())).to_string_lossy()
        }
    }
}
