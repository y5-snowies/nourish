//! Keyboard event types

use super::EventTrait;
use crate::{ffi, AsRaw, Context, FromRaw, Libinput};

/// State of a Key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    /// Key is pressed
    Pressed,
    /// Key is released
    Released,
}

/// Common functions for all Keyboard-Events implement.
pub trait KeyboardEventTrait: AsRaw<ffi::libinput_event_keyboard> + Context {
    ffi_func!(
    /// The event time for this event
    fn time, ffi::libinput_event_keyboard_get_time, u32);
    ffi_func!(
    /// The event time for this event in microseconds
    fn time_usec, ffi::libinput_event_keyboard_get_time_usec, u64);
    ffi_func!(
    /// The keycode that triggered this key event
    fn key, ffi::libinput_event_keyboard_get_key, u32);

    /// The state change of the key
    fn key_state(&self) -> KeyState {
        match unsafe { ffi::libinput_event_keyboard_get_key_state(self.as_raw() as *mut _) } {
            ffi::libinput_key_state_LIBINPUT_KEY_STATE_PRESSED => KeyState::Pressed,
            ffi::libinput_key_state_LIBINPUT_KEY_STATE_RELEASED => KeyState::Released,
            _ => panic!("libinput returned invalid 'libinput_key_state'"),
        }
    }

    /// Convert into a general `KeyboardEvent` again
    fn into_keyboard_event(self) -> KeyboardEvent
    where
        Self: Sized,
    {
        unsafe { KeyboardEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl<T: AsRaw<ffi::libinput_event_keyboard> + Context> KeyboardEventTrait for T {}

/// A keyboard related `Event`
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyboardEvent {
    /// An event related to pressing a key
    Key(KeyboardKeyEvent),
}

impl EventTrait for KeyboardEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            KeyboardEvent::Key(event) => event.as_raw_event(),
        }
    }
}

impl FromRaw<ffi::libinput_event_keyboard> for KeyboardEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_keyboard,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_keyboard_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_KEYBOARD_KEY => Some(KeyboardEvent::Key(
                KeyboardKeyEvent::try_from_raw(event, context)?,
            )),
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_keyboard, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown key event type")
    }
}

impl AsRaw<ffi::libinput_event_keyboard> for KeyboardEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_keyboard {
        match self {
            KeyboardEvent::Key(event) => event.as_raw(),
        }
    }
}

impl Context for KeyboardEvent {
    fn context(&self) -> &Libinput {
        match self {
            KeyboardEvent::Key(event) => event.context(),
        }
    }
}

ffi_event_struct!(
/// An event related to pressing a key
struct KeyboardKeyEvent, ffi::libinput_event_keyboard, ffi::libinput_event_keyboard_get_base_event);

impl KeyboardKeyEvent {
    ffi_func!(
    /// For the key of a `KeyboardKeyEvent` event, return the total number of keys
    /// pressed on all devices on the associated seat after the event was triggered.
    pub fn seat_key_count, ffi::libinput_event_keyboard_get_seat_key_count, u32);
}
