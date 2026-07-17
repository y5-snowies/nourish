//! Gesture event types

use super::EventTrait;
use crate::{ffi, AsRaw, Context, FromRaw, Libinput};

/// Common functions all Gesture-Events implement.
pub trait GestureEventTrait: AsRaw<ffi::libinput_event_gesture> + Context {
    ffi_func!(
    /// The event time for this event
    fn time, ffi::libinput_event_gesture_get_time, u32);
    ffi_func!(
    /// The event time for this event in microseconds
    fn time_usec, ffi::libinput_event_gesture_get_time_usec, u64);
    ffi_func!(
    /// Return the number of fingers used for a gesture.
    ///
    /// This can be used e.g. to differentiate between 3 or 4 finger swipes.
    ///
    /// This function can be called on all gesture events and the returned finger
    /// count value will not change during a sequence.
    fn finger_count, ffi::libinput_event_gesture_get_finger_count, i32);

    /// Convert into a general `GestureEvent` again
    fn into_gesture_event(self) -> GestureEvent
    where
        Self: Sized,
    {
        unsafe { GestureEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl<T: AsRaw<ffi::libinput_event_gesture> + Context> GestureEventTrait for T {}

/// A gesture related `Event`
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GestureEvent {
    /// A swipe gesture `Event`
    Swipe(GestureSwipeEvent),
    /// A pinch gesture `Event`
    Pinch(GesturePinchEvent),
    #[cfg(feature = "libinput_1_19")]
    /// A hold gesture `Event`
    Hold(GestureHoldEvent),
}

impl EventTrait for GestureEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            GestureEvent::Swipe(event) => event.as_raw_event(),
            GestureEvent::Pinch(event) => event.as_raw_event(),
            #[cfg(feature = "libinput_1_19")]
            GestureEvent::Hold(event) => event.as_raw_event(),
        }
    }
}

impl FromRaw<ffi::libinput_event_gesture> for GestureEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_gesture,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_gesture_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_UPDATE
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_END => Some(
                GestureEvent::Swipe(GestureSwipeEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_UPDATE
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_END => Some(
                GestureEvent::Pinch(GesturePinchEvent::try_from_raw(event, context)?),
            ),
            #[cfg(feature = "libinput_1_19")]
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_END => Some(GestureEvent::Hold(
                GestureHoldEvent::try_from_raw(event, context)?,
            )),
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_gesture, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown gesture event type")
    }
}

impl AsRaw<ffi::libinput_event_gesture> for GestureEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_gesture {
        match self {
            GestureEvent::Swipe(event) => event.as_raw(),
            GestureEvent::Pinch(event) => event.as_raw(),
            #[cfg(feature = "libinput_1_19")]
            GestureEvent::Hold(event) => event.as_raw(),
        }
    }
}

impl Context for GestureEvent {
    fn context(&self) -> &Libinput {
        match self {
            GestureEvent::Swipe(event) => event.context(),
            GestureEvent::Pinch(event) => event.context(),
            #[cfg(feature = "libinput_1_19")]
            GestureEvent::Hold(event) => event.context(),
        }
    }
}

/// Common functions for Gesture-Events having coordinates.
pub trait GestureEventCoordinates: AsRaw<ffi::libinput_event_gesture> {
    ffi_func!(
    /// Return the delta between the last event and the current event.
    ///
    /// If a device employs pointer acceleration, the delta returned by this
    /// function is the accelerated delta.
    ///
    /// Relative motion deltas are normalized to represent those of a device with
    /// 1000dpi resolution. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details.
    fn dx, ffi::libinput_event_gesture_get_dx, f64);
    ffi_func!(
    /// Return the relative delta of the unaccelerated motion vector of the
    /// current event.
    ///
    /// Relative unaccelerated motion deltas are normalized to represent those of
    /// a device with 1000dpi resolution. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details. Note that unaccelerated events are not equivalent to
    /// 'raw' events as read from the device.
    ///
    /// Any rotation applied to the device also applies to gesture motion (see
    /// `rotation_set_angle`).
    fn dx_unaccelerated, ffi::libinput_event_gesture_get_dx_unaccelerated, f64);
    ffi_func!(
    /// Return the delta between the last event and the current event.
    ///
    /// If a device employs pointer acceleration, the delta returned by this
    /// function is the accelerated delta.
    ///
    /// Relative motion deltas are normalized to represent those of a device with
    /// 1000dpi resolution. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details.
    fn dy, ffi::libinput_event_gesture_get_dy, f64);
    ffi_func!(
    /// Return the relative delta of the unaccelerated motion vector of the
    /// current event.
    ///
    /// Relative unaccelerated motion deltas are normalized to represent those of
    /// a device with 1000dpi resolution. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details. Note that unaccelerated events are not equivalent to
    /// 'raw' events as read from the device.
    ///
    /// Any rotation applied to the device also applies to gesture motion (see
    /// `rotation_set_angle`).
    fn dy_unaccelerated, ffi::libinput_event_gesture_get_dy_unaccelerated, f64);
}

/// Common functions for events noting the end of a gesture
pub trait GestureEndEvent: AsRaw<ffi::libinput_event_gesture> {
    ffi_func!(
    /// Return if the gesture ended normally, or if it was cancelled.
    fn cancelled, ffi::libinput_event_gesture_get_cancelled, bool);
}

/// Events for swipe gestures
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GestureSwipeEvent {
    /// Swipe gesture began
    Begin(GestureSwipeBeginEvent),
    /// In-progress swipe gesture updated
    Update(GestureSwipeUpdateEvent),
    /// Swipe gesture ended
    End(GestureSwipeEndEvent),
}

/// Common functions for swipe gesture events
pub trait GestureSwipeEventTrait: AsRaw<ffi::libinput_event_gesture> + Context {
    /// Convert into a general `GestureSwipeEvent`
    fn into_gesture_swipe_event(self) -> GestureSwipeEvent
    where
        Self: Sized,
    {
        unsafe { GestureSwipeEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl GestureSwipeEventTrait for GestureSwipeEvent {}

impl EventTrait for GestureSwipeEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            GestureSwipeEvent::Begin(event) => event.as_raw_event(),
            GestureSwipeEvent::Update(event) => event.as_raw_event(),
            GestureSwipeEvent::End(event) => event.as_raw_event(),
        }
    }
}

impl FromRaw<ffi::libinput_event_gesture> for GestureSwipeEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_gesture,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_gesture_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_BEGIN => Some(
                GestureSwipeEvent::Begin(GestureSwipeBeginEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_UPDATE => Some(
                GestureSwipeEvent::Update(GestureSwipeUpdateEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_END => Some(
                GestureSwipeEvent::End(GestureSwipeEndEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_UPDATE
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_END => {
                panic!("Tried to make GestureSwipeEvent from Pinch event")
            }
            #[cfg(feature = "libinput_1_19")]
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_END => {
                panic!("Tried to make GestureSwipeEvent from Hold event")
            }
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_gesture, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown gesture event type")
    }
}

impl AsRaw<ffi::libinput_event_gesture> for GestureSwipeEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_gesture {
        match self {
            GestureSwipeEvent::Begin(event) => event.as_raw(),
            GestureSwipeEvent::Update(event) => event.as_raw(),
            GestureSwipeEvent::End(event) => event.as_raw(),
        }
    }
}

impl Context for GestureSwipeEvent {
    fn context(&self) -> &Libinput {
        match self {
            GestureSwipeEvent::Begin(event) => event.context(),
            GestureSwipeEvent::Update(event) => event.context(),
            GestureSwipeEvent::End(event) => event.context(),
        }
    }
}

/// Events for pinch gestures
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GesturePinchEvent {
    /// Pinch gesture began
    Begin(GesturePinchBeginEvent),
    /// In-progress pinch gesture updated
    Update(GesturePinchUpdateEvent),
    /// Pinch gesture ended
    End(GesturePinchEndEvent),
}

impl EventTrait for GesturePinchEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            GesturePinchEvent::Begin(event) => event.as_raw_event(),
            GesturePinchEvent::Update(event) => event.as_raw_event(),
            GesturePinchEvent::End(event) => event.as_raw_event(),
        }
    }
}

/// Common functions for pinch gesture events
pub trait GesturePinchEventTrait: AsRaw<ffi::libinput_event_gesture> + Context {
    ffi_func!(
    /// Return the absolute scale of a pinch gesture, the scale is the division of
    /// the current distance between the fingers and the distance at the start of
    /// the gesture.
    ///
    /// The scale begins at 1.0, and if e.g. the fingers moved together by 50%
    /// then the scale will become 0.5, if they move twice as far apart as
    /// initially the scale becomes 2.0, etc.
    ///
    /// For gesture events that are of type `GesturePinchBeginEvent`, this function
    /// returns 1.0.
    /// For gesture events that are of type `GesturePinchEndEvent`, this function
    /// returns the scale value of the most recent `GesturePinchUpdateEvent` event
    /// (if any) or 1.0 otherwise.
    fn scale, ffi::libinput_event_gesture_get_scale, f64);

    /// Convert into a general `GesturePinchEvent`
    fn into_gesture_pinch_event(self) -> GesturePinchEvent
    where
        Self: Sized,
    {
        unsafe { GesturePinchEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl GesturePinchEventTrait for GesturePinchEvent {}

impl FromRaw<ffi::libinput_event_gesture> for GesturePinchEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_gesture,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_gesture_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_UPDATE
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_END => {
                panic!("Tried to make GesturePinchEvent from Swipe event")
            }
            #[cfg(feature = "libinput_1_19")]
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_END => {
                panic!("Tried to make GestureSwipeEvent from Hold event")
            }
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_BEGIN => Some(
                GesturePinchEvent::Begin(GesturePinchBeginEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_UPDATE => Some(
                GesturePinchEvent::Update(GesturePinchUpdateEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_END => Some(
                GesturePinchEvent::End(GesturePinchEndEvent::try_from_raw(event, context)?),
            ),
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_gesture, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown gesture event type")
    }
}

impl AsRaw<ffi::libinput_event_gesture> for GesturePinchEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_gesture {
        match self {
            GesturePinchEvent::Begin(event) => event.as_raw(),
            GesturePinchEvent::Update(event) => event.as_raw(),
            GesturePinchEvent::End(event) => event.as_raw(),
        }
    }
}

impl Context for GesturePinchEvent {
    fn context(&self) -> &Libinput {
        match self {
            GesturePinchEvent::Begin(event) => event.context(),
            GesturePinchEvent::Update(event) => event.context(),
            GesturePinchEvent::End(event) => event.context(),
        }
    }
}

#[cfg(feature = "libinput_1_19")]
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
/// Events for hold gestures
pub enum GestureHoldEvent {
    /// Hold gesture began
    Begin(GestureHoldBeginEvent),
    /// Hold gesture ended
    End(GestureHoldEndEvent),
}

#[cfg(feature = "libinput_1_19")]
impl EventTrait for GestureHoldEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            GestureHoldEvent::Begin(event) => event.as_raw_event(),
            GestureHoldEvent::End(event) => event.as_raw_event(),
        }
    }
}

#[cfg(feature = "libinput_1_19")]
/// Common functions for hold gesture events
pub trait GestureHoldEventTrait: AsRaw<ffi::libinput_event_gesture> + Context {
    /// Convert into a general `GesturePinchEvent`
    fn into_gesture_hold_event(self) -> GestureHoldEvent
    where
        Self: Sized,
    {
        unsafe { GestureHoldEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

#[cfg(feature = "libinput_1_19")]
impl GestureHoldEventTrait for GestureHoldEvent {}

#[cfg(feature = "libinput_1_19")]
impl FromRaw<ffi::libinput_event_gesture> for GestureHoldEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_gesture,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_gesture_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_UPDATE
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_SWIPE_END => {
                panic!("Tried to make GesturePinchEvent from Swipe event")
            }
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_BEGIN
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_UPDATE
            | ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_PINCH_END => {
                panic!("Tried to make GestureSwipeEvent from Pinch event")
            }
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_BEGIN => Some(
                GestureHoldEvent::Begin(GestureHoldBeginEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_GESTURE_HOLD_END => Some(
                GestureHoldEvent::End(GestureHoldEndEvent::try_from_raw(event, context)?),
            ),
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_gesture, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown gesture event type")
    }
}

#[cfg(feature = "libinput_1_19")]
impl AsRaw<ffi::libinput_event_gesture> for GestureHoldEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_gesture {
        match self {
            GestureHoldEvent::Begin(event) => event.as_raw(),
            GestureHoldEvent::End(event) => event.as_raw(),
        }
    }
}

#[cfg(feature = "libinput_1_19")]
impl Context for GestureHoldEvent {
    fn context(&self) -> &Libinput {
        match self {
            GestureHoldEvent::Begin(event) => event.context(),
            GestureHoldEvent::End(event) => event.context(),
        }
    }
}

ffi_event_struct!(
/// Swipe gesture began
struct GestureSwipeBeginEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

impl GestureSwipeEventTrait for GestureSwipeBeginEvent {}

ffi_event_struct!(
/// In-progress swipe gesture updated
struct GestureSwipeUpdateEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

impl GestureSwipeEventTrait for GestureSwipeUpdateEvent {}

impl GestureEventCoordinates for GestureSwipeUpdateEvent {}

ffi_event_struct!(
/// Swipe gesture ended
struct GestureSwipeEndEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

impl GestureEndEvent for GestureSwipeEndEvent {}

impl GestureSwipeEventTrait for GestureSwipeEndEvent {}

ffi_event_struct!(
/// Pinch gesture began
struct GesturePinchBeginEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

impl GesturePinchEventTrait for GesturePinchBeginEvent {}

ffi_event_struct!(
/// In-progress pinch gesture updated
struct GesturePinchUpdateEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

impl GesturePinchEventTrait for GesturePinchUpdateEvent {}

impl GestureEventCoordinates for GesturePinchUpdateEvent {}

impl GesturePinchUpdateEvent {
    ffi_func!(
    /// Return the angle delta in degrees between the last and the current
    /// `GesturePinchUpdateEvent`.
    ///
    /// The angle delta is defined as the change in angle of the line formed by
    /// the 2 fingers of a pinch gesture. Clockwise rotation is represented by a
    /// positive delta, counter-clockwise by a negative delta. If e.g. the fingers
    /// are on the 12 and 6 location of a clock face plate and they move to the 1
    /// resp. 7 location in a single event then the angle delta is 30 degrees.
    ///
    /// If more than two fingers are present, the angle represents the rotation
    /// around the center of gravity. The calculation of the center of gravity is
    /// implementation-dependent.
    pub fn angle_delta, ffi::libinput_event_gesture_get_angle_delta, f64);
}

ffi_event_struct!(
/// Pinch gesture ended
struct GesturePinchEndEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

impl GesturePinchEventTrait for GesturePinchEndEvent {}

impl GestureEndEvent for GesturePinchEndEvent {}

#[cfg(feature = "libinput_1_19")]
ffi_event_struct!(
/// Hold gesture began
struct GestureHoldBeginEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

#[cfg(feature = "libinput_1_19")]
impl GestureHoldEventTrait for GestureHoldBeginEvent {}

#[cfg(feature = "libinput_1_19")]
ffi_event_struct!(
/// Hold gesture ended
struct GestureHoldEndEvent, ffi::libinput_event_gesture, ffi::libinput_event_gesture_get_base_event);

#[cfg(feature = "libinput_1_19")]
impl GestureEndEvent for GestureHoldEndEvent {}

#[cfg(feature = "libinput_1_19")]
impl GestureHoldEventTrait for GestureHoldEndEvent {}
