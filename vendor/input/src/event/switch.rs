//! Switch event types

use super::EventTrait;
use crate::{ffi, AsRaw, Context, FromRaw, Libinput};

/// Common functions all Switch-Events implement.
pub trait SwitchEventTrait: AsRaw<ffi::libinput_event_switch> + Context {
    ffi_func!(
    /// The event time for this event
    fn time, ffi::libinput_event_switch_get_time, u32);
    ffi_func!(
    /// The event time for this event in microseconds
    fn time_usec, ffi::libinput_event_switch_get_time_usec, u64);

    /// Convert into a general `SwitchEvent` again
    fn into_switch_event(self) -> SwitchEvent
    where
        Self: Sized,
    {
        unsafe { SwitchEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl<T: AsRaw<ffi::libinput_event_switch> + Context> SwitchEventTrait for T {}

/// A switch related `Event`
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SwitchEvent {
    /// An event related a switch, that was toggled
    Toggle(SwitchToggleEvent),
}

impl EventTrait for SwitchEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            SwitchEvent::Toggle(event) => event.as_raw_event(),
        }
    }
}

impl FromRaw<ffi::libinput_event_switch> for SwitchEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_switch,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_switch_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_SWITCH_TOGGLE => Some(SwitchEvent::Toggle(
                SwitchToggleEvent::try_from_raw(event, context)?,
            )),
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_switch, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown switch event type")
    }
}

impl AsRaw<ffi::libinput_event_switch> for SwitchEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_switch {
        match self {
            SwitchEvent::Toggle(event) => event.as_raw(),
        }
    }
}

impl Context for SwitchEvent {
    fn context(&self) -> &Libinput {
        match self {
            SwitchEvent::Toggle(event) => event.context(),
        }
    }
}

/// Types of Switches
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum Switch {
    /// The laptop lid was closed when the `SwitchState` is
    /// `On`, or was opened when it is `Off`
    Lid = ffi::libinput_switch_LIBINPUT_SWITCH_LID,
    /// This switch indicates whether the device is in normal laptop mode
    /// or behaves like a tablet-like device where the primary
    /// interaction is usually a touch screen. When in tablet mode, the
    /// keyboard and touchpad are usually inaccessible.
    ///
    /// If the switch is in state `SwitchState::Off`, the
    /// device is in laptop mode. If the switch is in state
    /// `SwitchState::On`, the device is in tablet mode and the
    /// keyboard or touchpad may not be  accessible.
    ///
    /// It is up to the caller to identify which devices are inaccessible
    /// `in tablet mode.
    TabletMode = ffi::libinput_switch_LIBINPUT_SWITCH_TABLET_MODE,
}

/// State of a Switch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwitchState {
    /// Switch is off
    Off,
    /// Switch is on
    On,
}

ffi_event_struct!(
/// An event related a switch, that was toggled
struct SwitchToggleEvent, ffi::libinput_event_switch, ffi::libinput_event_switch_get_base_event);

impl SwitchToggleEvent {
    /// Return the switch that triggered this event.
    ///
    /// A return value of `None` means, the switch type is not known
    pub fn switch(&self) -> Option<Switch> {
        match unsafe { ffi::libinput_event_switch_get_switch(self.as_raw_mut()) } {
            ffi::libinput_switch_LIBINPUT_SWITCH_LID => Some(Switch::Lid),
            ffi::libinput_switch_LIBINPUT_SWITCH_TABLET_MODE => Some(Switch::TabletMode),
            _x => {
                #[cfg(feature = "log")]
                log::warn!("Unknown Switch type returned by libinput: {}", _x);
                None
            }
        }
    }

    /// Return the switch state that triggered this event.
    pub fn switch_state(&self) -> SwitchState {
        match unsafe { ffi::libinput_event_switch_get_switch_state(self.as_raw_mut()) } {
            ffi::libinput_switch_state_LIBINPUT_SWITCH_STATE_OFF => SwitchState::Off,
            ffi::libinput_switch_state_LIBINPUT_SWITCH_STATE_ON => SwitchState::On,
            _ => panic!("libinput returned invalid 'libinput_switch_state'"),
        }
    }
}
