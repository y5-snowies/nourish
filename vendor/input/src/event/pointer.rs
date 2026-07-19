//! Pointer event types
#![allow(deprecated)]

use super::EventTrait;
use crate::{ffi, AsRaw, Context, FromRaw, Libinput};

/// Common functions for all Pointer-Events implement.
pub trait PointerEventTrait: AsRaw<ffi::libinput_event_pointer> + Context {
    ffi_func!(
    /// The event time for this event
    fn time, ffi::libinput_event_pointer_get_time, u32);
    ffi_func!(
    /// The event time for this event in microseconds
    fn time_usec, ffi::libinput_event_pointer_get_time_usec, u64);

    /// Convert into a general `TouchEvent` again
    fn into_pointer_event(self) -> PointerEvent
    where
        Self: Sized,
    {
        unsafe { PointerEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl<T: AsRaw<ffi::libinput_event_pointer> + Context> PointerEventTrait for T {}

/// A pointer related `Event`
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerEvent {
    /// An event related to moving a pointer
    Motion(PointerMotionEvent),
    /// An event related to absolute pointer movement
    MotionAbsolute(PointerMotionAbsoluteEvent),
    /// An event related to button pressed on a pointer device
    Button(PointerButtonEvent),
    /// An event related to moving axis on a pointer device
    #[cfg_attr(
        feature = "libinput_1_19",
        deprecated = "Use `PointerEvent::Scroll*` events instead"
    )]
    Axis(PointerAxisEvent),
    /// A scroll event from a wheel.
    #[cfg(feature = "libinput_1_19")]
    ScrollWheel(PointerScrollWheelEvent),
    /// A scroll event caused by the movement of one or more fingers on a device.
    #[cfg(feature = "libinput_1_19")]
    ScrollFinger(PointerScrollFingerEvent),
    /// A scroll event from a continuous scroll source, e.g. button scrolling.
    #[cfg(feature = "libinput_1_19")]
    ScrollContinuous(PointerScrollContinuousEvent),
}

impl EventTrait for PointerEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            PointerEvent::Motion(event) => event.as_raw_event(),
            PointerEvent::MotionAbsolute(event) => event.as_raw_event(),
            PointerEvent::Button(event) => event.as_raw_event(),
            PointerEvent::Axis(event) => event.as_raw_event(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollWheel(event) => event.as_raw_event(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollFinger(event) => event.as_raw_event(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollContinuous(event) => event.as_raw_event(),
        }
    }
}

impl FromRaw<ffi::libinput_event_pointer> for PointerEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_pointer,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_pointer_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_MOTION => Some(PointerEvent::Motion(
                PointerMotionEvent::try_from_raw(event, context)?,
            )),
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_MOTION_ABSOLUTE => {
                Some(PointerEvent::MotionAbsolute(
                    PointerMotionAbsoluteEvent::try_from_raw(event, context)?,
                ))
            }
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_BUTTON => Some(PointerEvent::Button(
                PointerButtonEvent::try_from_raw(event, context)?,
            )),
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_AXIS => Some(PointerEvent::Axis(
                PointerAxisEvent::try_from_raw(event, context)?,
            )),
            #[cfg(feature = "libinput_1_19")]
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_SCROLL_WHEEL => Some(
                PointerEvent::ScrollWheel(PointerScrollWheelEvent::try_from_raw(event, context)?),
            ),
            #[cfg(feature = "libinput_1_19")]
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_SCROLL_FINGER => Some(
                PointerEvent::ScrollFinger(PointerScrollFingerEvent::try_from_raw(event, context)?),
            ),
            #[cfg(feature = "libinput_1_19")]
            ffi::libinput_event_type_LIBINPUT_EVENT_POINTER_SCROLL_CONTINUOUS => {
                Some(PointerEvent::ScrollContinuous(
                    PointerScrollContinuousEvent::try_from_raw(event, context)?,
                ))
            }
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_pointer, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown pointer event type")
    }
}

impl AsRaw<ffi::libinput_event_pointer> for PointerEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_pointer {
        match self {
            PointerEvent::Motion(event) => event.as_raw(),
            PointerEvent::MotionAbsolute(event) => event.as_raw(),
            PointerEvent::Button(event) => event.as_raw(),
            PointerEvent::Axis(event) => event.as_raw(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollWheel(event) => event.as_raw(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollFinger(event) => event.as_raw(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollContinuous(event) => event.as_raw(),
        }
    }
}

impl Context for PointerEvent {
    fn context(&self) -> &Libinput {
        match self {
            PointerEvent::Motion(event) => event.context(),
            PointerEvent::MotionAbsolute(event) => event.context(),
            PointerEvent::Button(event) => event.context(),
            PointerEvent::Axis(event) => event.context(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollWheel(event) => event.context(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollFinger(event) => event.context(),
            #[cfg(feature = "libinput_1_19")]
            PointerEvent::ScrollContinuous(event) => event.context(),
        }
    }
}

ffi_event_struct!(
/// An event related to moving a pointer
struct PointerMotionEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

impl PointerMotionEvent {
    ffi_func!(
    /// Return the delta between the last event and the current event.
    ///
    /// If a device employs pointer acceleration, the delta returned by this
    /// function is the accelerated delta.
    ///
    /// Relative motion deltas are to be interpreted as pixel movement of a
    /// standardized mouse. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details.
    pub fn dx, ffi::libinput_event_pointer_get_dx, f64);
    ffi_func!(
    /// Return the relative delta of the unaccelerated motion vector of the
    /// current event.
    ///
    /// Relative unaccelerated motion deltas are raw device coordinates. Note that
    /// these coordinates are subject to the device's native resolution. Touchpad
    /// coordinates represent raw device coordinates in the X resolution of the
    /// touchpad. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details.
    ///
    /// Any rotation applied to the device also applies to unaccelerated motion
    /// (see `Device::rotation_set_angle`).
    pub fn dx_unaccelerated, ffi::libinput_event_pointer_get_dx_unaccelerated, f64);
    ffi_func!(
    /// Return the delta between the last event and the current event.
    ///
    /// If a device employs pointer acceleration, the delta returned by this
    /// function is the accelerated delta.
    ///
    /// Relative motion deltas are to be interpreted as pixel movement of a
    /// standardized mouse. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details.
    pub fn dy, ffi::libinput_event_pointer_get_dy, f64);
    ffi_func!(
    /// Return the relative delta of the unaccelerated motion vector of the
    /// current event.
    ///
    /// Relative unaccelerated motion deltas are raw device coordinates. Note that
    /// these coordinates are subject to the device's native resolution. Touchpad
    /// coordinates represent raw device coordinates in the X resolution of the
    /// touchpad. See [Normalization of relative motion](https://wayland.freedesktop.org/libinput/doc/latest/motion_normalization.html)
    /// for more details.
    ///
    /// Any rotation applied to the device also applies to unaccelerated motion
    /// (see `Device::rotation_set_angle`).
    pub fn dy_unaccelerated, ffi::libinput_event_pointer_get_dy_unaccelerated, f64);
}

ffi_event_struct!(
/// An event related to absolute pointer movement
struct PointerMotionAbsoluteEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

impl PointerMotionAbsoluteEvent {
    ffi_func!(
    /// Return the current absolute x coordinate of the pointer event, in mm from
    /// the top left corner of the device.
    ///
    /// To get the corresponding output screen coordinate, use
    /// `absolute_x_transformed`.
    pub fn absolute_x, ffi::libinput_event_pointer_get_absolute_x, f64);
    ffi_func!(
    /// Return the current absolute y coordinate of the pointer event, in mm from
    /// the top left corner of the device.
    ///
    /// To get the corresponding output screen coordinate, use
    /// `absolute_y_transformed`.
    pub fn absolute_y, ffi::libinput_event_pointer_get_absolute_y, f64);

    /// Return the current absolute x coordinate of the pointer event, transformed
    /// to screen coordinates.
    ///
    /// ## Arguments
    ///
    /// - width - The current output screen width
    pub fn absolute_x_transformed(&self, width: u32) -> f64 {
        unsafe { ffi::libinput_event_pointer_get_absolute_x_transformed(self.as_raw_mut(), width) }
    }

    /// Return the current absolute y coordinate of the pointer event, transformed
    /// to screen coordinates.
    ///
    /// ## Arguments
    ///
    /// - height - The current output screen height
    pub fn absolute_y_transformed(&self, height: u32) -> f64 {
        unsafe { ffi::libinput_event_pointer_get_absolute_y_transformed(self.as_raw_mut(), height) }
    }
}

/// State of a Button
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonState {
    /// Button is pressed
    Pressed,
    /// Button is released
    Released,
}

ffi_event_struct!(
/// An event related to button pressed on a pointer device
struct PointerButtonEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

impl PointerButtonEvent {
    ffi_func!(
    /// Return the button that triggered this event.
    pub fn button, ffi::libinput_event_pointer_get_button, u32);
    ffi_func!(
    /// For the button returns the total number of buttons pressed on all devices
    /// on the associated seat after the event was triggered.
    pub fn seat_button_count, ffi::libinput_event_pointer_get_seat_button_count, u32);

    /// Return the button state that triggered this event.
    pub fn button_state(&self) -> ButtonState {
        match unsafe { ffi::libinput_event_pointer_get_button_state(self.as_raw_mut()) } {
            ffi::libinput_button_state_LIBINPUT_BUTTON_STATE_PRESSED => ButtonState::Pressed,
            ffi::libinput_button_state_LIBINPUT_BUTTON_STATE_RELEASED => ButtonState::Released,
            _ => panic!("libinput returned invalid 'libinput_button_state'"),
        }
    }
}

/// The source for a `PointerAxisEvent`.
#[cfg_attr(
    feature = "libinput_1_19",
    deprecated = "Use `PointerEvent::Scroll*` events instead"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisSource {
    /// The event is caused by the rotation of a wheel.
    Wheel,
    /// The event is caused by the movement of one or more fingers on a device.
    Finger,
    /// The event is caused by the motion of some device.
    Continuous,
    /// The event is caused by the tilting of a mouse wheel rather than its rotation.
    ///
    /// This method is commonly used on mice without separate horizontal scroll wheels.
    #[cfg_attr(
        feature = "libinput_1_19",
        deprecated = "No device has ever sent this source."
    )]
    WheelTilt,
}

/// Axes on a device with the pointer capability that are not  x or y coordinates.
///
/// The two scroll axes `Vertical` and `Horizontal` are engaged separately,
/// depending on the device. libinput provides some scroll direction locking but
/// it is up to the caller to determine which axis is needed and appropriate in
/// the current interaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Vertical axis
    Vertical,
    /// Horizontal axis
    Horizontal,
}

ffi_event_struct!(
/// An event related to moving axis on a pointer device
#[cfg_attr(feature = "libinput_1_19", deprecated = "Use `PointerEvent::Scroll*` events instead")]
struct PointerAxisEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

impl PointerAxisEvent {
    /// Check if the event has a valid value for the given axis.
    ///
    /// If this function returns true for an axis and `axis_value` returns a
    /// value of 0, the event is a scroll stop event.
    #[cfg_attr(
        feature = "libinput_1_19",
        deprecated = "Use `PointerScrollEvent::has_axis` instead"
    )]
    pub fn has_axis(&self, axis: Axis) -> bool {
        unsafe {
            ffi::libinput_event_pointer_has_axis(
                self.as_raw_mut(),
                match axis {
                    Axis::Vertical => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL
                    }
                    Axis::Horizontal => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL
                    }
                },
            ) != 0
        }
    }

    /// Return the source for a given axis event.
    ///
    /// Axis events (scroll events) can be caused by a hardware item such as a
    /// scroll wheel or emulated from other input sources, such as two-finger or
    /// edge scrolling on a touchpad.
    ///
    /// If the source is `Finger`, libinput guarantees that a scroll sequence is
    /// terminated with a scroll value of 0. A caller may use this information to
    /// decide on whether kinetic scrolling should be triggered on this scroll
    /// sequence. The coordinate system is identical to the cursor movement, i.e.
    /// a scroll value of 1 represents the equivalent relative motion of 1.
    ///
    /// If the source is `Wheel`, no terminating event is guaranteed (though it
    /// may happen). Scrolling is in discrete steps, the value is the angle the
    /// wheel moved in degrees. The default is 15 degrees per wheel click, but
    /// some mice may have differently grained wheels. It is up to the caller how
    /// to interpret such different step sizes.
    ///
    /// If the source is `Continuous`, no terminating event is guaranteed (though
    /// it may happen). The coordinate system is identical to the cursor movement,
    /// i.e. a scroll value of 1 represents the equivalent relative motion of 1.
    ///
    /// If the source is `WheelTilt`, no terminating event is guaranteed (though
    /// it may happen). Scrolling is in discrete steps and there is no physical
    /// equivalent for the value returned here. For backwards compatibility, the
    /// value returned by this function is identical to a single mouse wheel
    /// rotation by this device (see the documentation for `WheelTilt` above).
    /// Callers should not use this value but instead exclusively refer to the
    //. value returned by `axis_value_discrete`.
    #[cfg_attr(
        feature = "libinput_1_19",
        deprecated = "Use `PointerScroll*` events instead"
    )]
    pub fn axis_source(&self) -> AxisSource {
        match unsafe { ffi::libinput_event_pointer_get_axis_source(self.as_raw_mut()) } {
            ffi::libinput_pointer_axis_source_LIBINPUT_POINTER_AXIS_SOURCE_WHEEL => {
                AxisSource::Wheel
            }
            ffi::libinput_pointer_axis_source_LIBINPUT_POINTER_AXIS_SOURCE_FINGER => {
                AxisSource::Finger
            }
            ffi::libinput_pointer_axis_source_LIBINPUT_POINTER_AXIS_SOURCE_CONTINUOUS => {
                AxisSource::Continuous
            }
            ffi::libinput_pointer_axis_source_LIBINPUT_POINTER_AXIS_SOURCE_WHEEL_TILT => {
                AxisSource::WheelTilt
            }
            // Axis Event is deprecated, no new variants will be added
            _ => unreachable!(),
        }
    }

    /// Return the axis value of the given axis.
    ///
    /// The interpretation of the value depends on the axis. For the two scrolling
    /// axes `Vertical` and `Horizontal`, the value of the event is in relative
    /// scroll units, with the positive direction being down or right,
    /// respectively. For the interpretation of the value, see `axis_source`.
    ///
    /// If `has_axis` returns `false` for an axis, this function returns 0 for
    /// that axis.
    #[cfg_attr(
        feature = "libinput_1_19",
        deprecated = "Use `PointerScrollEvent::scroll_value` instead"
    )]
    pub fn axis_value(&self, axis: Axis) -> f64 {
        unsafe {
            ffi::libinput_event_pointer_get_axis_value(
                self.as_raw_mut(),
                match axis {
                    Axis::Vertical => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL
                    }
                    Axis::Horizontal => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL
                    }
                },
            )
        }
    }

    /// Return the axis value in discrete steps for a given axis event.
    ///
    /// How a value translates into a discrete step depends on the source.
    ///
    /// If the source is `Wheel`, the discrete value correspond to the number of
    /// physical mouse wheel clicks.
    ///
    /// If the source is `Continuous` or `Finger`, the discrete value is always
    /// `None`.
    #[cfg_attr(
        feature = "libinput_1_19",
        deprecated = "Use `PointerScrollWheelEvent::scroll_value_v120` instead"
    )]
    pub fn axis_value_discrete(&self, axis: Axis) -> Option<f64> {
        match self.axis_source() {
            AxisSource::Continuous | AxisSource::Finger => None,
            _ => Some(unsafe {
                ffi::libinput_event_pointer_get_axis_value_discrete(
                    self.as_raw_mut(),
                    match axis {
                        Axis::Vertical => {
                            ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL
                        }
                        Axis::Horizontal => {
                            ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL
                        }
                    },
                )
            }),
        }
    }
}

#[cfg(feature = "libinput_1_19")]
ffi_event_struct!(
/// An event related to moving a scroll whell on a pointer device
struct PointerScrollWheelEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

#[cfg(feature = "libinput_1_19")]
ffi_event_struct!(
/// An event related to moving a finger on a pointer device
struct PointerScrollFingerEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

#[cfg(feature = "libinput_1_19")]
ffi_event_struct!(
/// An event related to a continuous scroll source on a pointer device
struct PointerScrollContinuousEvent, ffi::libinput_event_pointer, ffi::libinput_event_pointer_get_base_event);

#[cfg(feature = "libinput_1_19")]
/// Common functions of PointerScroll type events
pub trait PointerScrollEvent: AsRaw<ffi::libinput_event_pointer> {
    /// Check if the event has a valid value for the given axis.
    ///
    /// If this function returns true for an axis and `axis_value` returns a
    /// value of 0, the event is a scroll stop event.
    fn has_axis(&self, axis: Axis) -> bool {
        unsafe {
            ffi::libinput_event_pointer_has_axis(
                self.as_raw_mut(),
                match axis {
                    Axis::Vertical => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL
                    }
                    Axis::Horizontal => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL
                    }
                },
            ) != 0
        }
    }

    /// Return the axis value of the given axis.
    ///
    /// The interpretation of the value depends on the axis. For the two scrolling axes [`Axis::Vertical`] and [`Axis::Horizontal`],
    /// the value of the event is in relative scroll units, with the positive direction being down or right,
    /// respectively. If [`PointerScrollEvent::has_axis`] returns false for an axis, this function returns 0 for that axis.
    ///
    /// If the event is a [`PointerScrollFingerEvent`], libinput guarantees that a scroll sequence is terminated with a scroll value of 0.
    /// A caller may use this information to decide on whether kinetic scrolling should be triggered on this scroll sequence.
    /// The coordinate system is identical to the cursor movement, i.e. a scroll value of 1 represents the equivalent relative motion of 1.
    ///
    /// If the event is a [`PointerScrollWheelEvent`], no terminating event is guaranteed (though it may happen).
    /// Scrolling is in discrete steps, the value is the angle the wheel moved in degrees. The default is 15 degrees per wheel click,
    /// but some mice may have differently grained wheels. It is up to the caller how to interpret such different step sizes.
    /// Callers should use [`PointerScrollWheelEvent::scroll_value_v120`] for a simpler API of handling scroll wheel events of different step sizes.
    ///
    /// If the event is a [`PointerScrollContinuousEvent`], libinput guarantees that a scroll sequence is terminated with a scroll value of 0.
    /// The coordinate system is identical to the cursor movement, i.e. a scroll value of 1 represents the equivalent relative motion of 1.
    fn scroll_value(&self, axis: Axis) -> f64 {
        unsafe {
            ffi::libinput_event_pointer_get_scroll_value(
                self.as_raw_mut(),
                match axis {
                    Axis::Vertical => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL
                    }
                    Axis::Horizontal => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL
                    }
                },
            )
        }
    }
}

#[cfg(feature = "libinput_1_19")]
impl PointerScrollEvent for PointerScrollWheelEvent {}
#[cfg(feature = "libinput_1_19")]
impl PointerScrollEvent for PointerScrollFingerEvent {}
#[cfg(feature = "libinput_1_19")]
impl PointerScrollEvent for PointerScrollContinuousEvent {}

#[cfg(feature = "libinput_1_19")]
impl PointerScrollWheelEvent {
    /// Return the axis value as a v120-normalized value, that represents the movement in logical mouse wheel clicks, normalized to the -120..+120 range.
    ///
    /// A value that is a fraction of ±120 indicates a wheel movement less than one logical click,
    /// a caller should either scroll by the respective fraction of the normal scroll distance or accumulate
    /// that value until a multiple of 120 is reached.
    ///
    /// For most callers, this is the preferred way of handling high-resolution scroll events.
    ///
    /// The normalized v120 value does not take device-specific physical angles or distances into account,
    /// i.e. a wheel with a click angle of 20 degrees produces only 18 logical clicks per 360 degree rotation,
    /// a wheel with a click angle of 15 degrees produces 24 logical clicks per 360 degree rotation.
    /// Where the physical angle matters, use [`PointerScrollEvent::scroll_value`] instead.
    ///
    /// The magic number 120 originates from the [Windows Vista Mouse Wheel design document](http://download.microsoft.com/download/b/d/1/bd1f7ef4-7d72-419e-bc5c-9f79ad7bb66e/wheel.docx).
    pub fn scroll_value_v120(&self, axis: Axis) -> f64 {
        unsafe {
            ffi::libinput_event_pointer_get_scroll_value_v120(
                self.as_raw_mut(),
                match axis {
                    Axis::Vertical => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL
                    }
                    Axis::Horizontal => {
                        ffi::libinput_pointer_axis_LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL
                    }
                },
            )
        }
    }
}
