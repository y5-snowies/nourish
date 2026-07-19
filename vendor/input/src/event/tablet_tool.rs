//! Tablet tool event types

use super::{pointer::ButtonState, EventTrait};
use crate::{ffi, AsRaw, Context, FromRaw, Libinput};

mod tool;
pub use self::tool::*;

/// Common functions all TabletTool-Events implement.
pub trait TabletToolEventTrait: AsRaw<ffi::libinput_event_tablet_tool> + Context {
    ffi_func!(
    /// The event time for this event
    fn time, ffi::libinput_event_tablet_tool_get_time, u32);
    ffi_func!(
    /// The event time for this event in microseconds
    fn time_usec, ffi::libinput_event_tablet_tool_get_time_usec, u64);
    ffi_func!(
    /// Check if the distance axis was updated in this event.
    ///
    /// For `TabletToolProximityEvent`s this function always returns `true`. For
    /// `TabletToolButtonEvent`s this function always returns `false`.
    fn distance_has_changed, ffi::libinput_event_tablet_tool_distance_has_changed, bool);
    ffi_func!(
    /// Returns the current distance from the tablet's sensor, normalized to the range
    /// [0, 1].
    ///
    /// If this axis does not exist on the current tool, this function returns 0.
    fn distance, ffi::libinput_event_tablet_tool_get_distance, f64);
    ffi_func!(
    /// Return the delta between the last event and the current event.
    ///
    /// If the tool employs pointer acceleration, the delta returned by this function is
    /// the accelerated delta.
    ///
    /// This value is in screen coordinate space, the delta is to be interpreted like
    /// the return value of `PointerMotionEvent::dx`. See
    /// [Relative motion for tablet tools](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-relative-motion)
    /// for more details.
    fn dx, ffi::libinput_event_tablet_tool_get_dx, f64);
    ffi_func!(
    /// Return the delta between the last event and the current event.
    ///
    /// If the tool employs pointer acceleration, the delta returned by this function is
    /// the accelerated delta.
    ///
    /// This value is in screen coordinate space, the delta is to be interpreted like
    /// the return value of `PointerMotionEvent::dy`. See
    /// [Relative motion for tablet tools](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-relative-motion)
    /// for more details.
    fn dy, ffi::libinput_event_tablet_tool_get_dy, f64);
    ffi_func!(
    /// Check if the pressure axis was updated in this event. For `TabletToolButtonEvent`s this function always returns `false`.
    fn pressure_has_changed, ffi::libinput_event_tablet_tool_pressure_has_changed, bool);
    ffi_func!(
    /// Returns the current pressure being applied on the tool in use, normalized to the
    /// range [0, 1].
    ///
    /// If this axis does not exist on the current tool, this function returns 0.
    fn pressure, ffi::libinput_event_tablet_tool_get_pressure, f64);
    ffi_func!(
    /// Check if the z-rotation axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn rotation_has_changed, ffi::libinput_event_tablet_tool_rotation_has_changed, bool);
    ffi_func!(
    /// Returns the current z rotation of the tool in degrees, clockwise from the tool's
    /// logical neutral position.
    ///
    /// For tools of type `TabletToolType::Mouse` and `TabletToolType::Lens` the logical
    /// neutral position is pointing to the current logical north of the tablet. For
    /// tools of type `TabletToolType::Brush`, the logical neutral position is with the
    /// buttons pointing up.
    ///
    /// If this axis does not exist on the current tool, this function returns 0.
    fn rotation, ffi::libinput_event_tablet_tool_get_rotation, f64);
    ffi_func!(
    /// Check if the slider axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn slider_has_changed, ffi::libinput_event_tablet_tool_slider_has_changed, bool);
    ffi_func!(
    /// Returns the current position of the slider on the tool, normalized to the range
    /// [-1, 1].
    ///
    /// The logical zero is the neutral position of the slider, or the logical center of
    /// the axis. This axis is available on e.g. the Wacom Airbrush.
    ///
    /// If this axis does not exist on the current tool, this function returns 0.
    fn slider_position, ffi::libinput_event_tablet_tool_get_slider_position, f64);
    ffi_func!(
    /// Check if the tilt x axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn tilt_x_has_changed, ffi::libinput_event_tablet_tool_tilt_x_has_changed, bool);
    ffi_func!(
    /// Check if the tilt y axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn tilt_y_has_changed, ffi::libinput_event_tablet_tool_tilt_y_has_changed, bool);
    ffi_func!(
    /// Returns the current tilt along the X axis of the tablet's current logical
    /// orientation, in degrees off the tablet's z axis.
    ///
    /// That is, if the tool is perfectly orthogonal to the tablet, the tilt angle is 0.
    /// When the top tilts towards the logical top/left of the tablet, the x/y tilt
    /// angles are negative, if the top tilts towards the logical bottom/right of the
    /// tablet, the x/y tilt angles are positive.
    ///
    /// If this axis does not exist on the current tool, this function returns 0.
    fn tilt_x, ffi::libinput_event_tablet_tool_get_tilt_x, f64);
    ffi_func!(
    /// Returns the current tilt along the Y axis of the tablet's current logical
    /// orientation, in degrees off the tablet's z axis.
    ///
    /// That is, if the tool is perfectly orthogonal to the tablet, the tilt angle is 0.
    /// When the top tilts towards the logical top/left of the tablet, the x/y tilt
    /// angles are negative, if the top tilts towards the logical bottom/right of the
    /// tablet, the x/y tilt angles are positive.
    ///
    /// If this axis does not exist on the current tool, this function returns 0.
    fn tilt_y, ffi::libinput_event_tablet_tool_get_tilt_y, f64);
    #[cfg(feature = "libinput_1_14")]
    ffi_func!(
    /// Check if the size major axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns false
    fn size_major_has_changed, ffi::libinput_event_tablet_tool_size_major_has_changed, bool);
    #[cfg(feature = "libinput_1_14")]
    ffi_func!(
    /// Check if the size minor axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns false
    fn size_minor_has_changed, ffi::libinput_event_tablet_tool_size_minor_has_changed, bool);
    #[cfg(feature = "libinput_1_14")]
    ffi_func!(
    /// Returns the current size in mm along the major axis of the touching ellipse.
    /// This axis is not necessarily aligned with either x or y, the rotation must
    /// be taken into account.
    ///
    /// Where no rotation is available on a tool, or where rotation is zero, the major
    /// axis aligns with the y axis and the minor axis with the x axis.
    ///
    /// If the axis does not exist on the current tool, this function returns 0.
    fn size_major, ffi::libinput_event_tablet_tool_get_size_major, f64);
    #[cfg(feature = "libinput_1_14")]
    ffi_func!(
    /// Returns the current size in mm along the minor axis of the touching ellipse.
    /// This axis is not necessarily aligned with either x or y, the rotation must
    /// be taken into account.
    ///
    /// Where no rotation is available on a tool, or where rotation is zero, the minor
    /// axis aligns with the y axis and the major axis with the x axis.
    ///
    /// If the axis does not exist on the current tool, this function returns 0.
    fn size_minor, ffi::libinput_event_tablet_tool_get_size_minor, f64);
    ffi_func!(
    /// Check if the wheel axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn wheel_has_changed, ffi::libinput_event_tablet_tool_wheel_has_changed, bool);
    ffi_func!(
    /// Return the delta for the wheel in degrees.
    fn wheel_delta, ffi::libinput_event_tablet_tool_get_wheel_delta, f64);
    ffi_func!(
    /// Return the delta for the wheel in discrete steps (e.g. wheel clicks).
    fn wheel_delta_discrete, ffi::libinput_event_tablet_tool_get_wheel_delta_discrete, f64);
    ffi_func!(
    /// Check if the x axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn x_has_changed, ffi::libinput_event_tablet_tool_x_has_changed, bool);
    ffi_func!(
    /// Check if the y axis was updated in this event.
    ///
    /// For `TabletToolButtonEvent`s this function always returns `false`.
    fn y_has_changed, ffi::libinput_event_tablet_tool_y_has_changed, bool);
    ffi_func!(
    /// Returns the X coordinate of the tablet tool, in mm from the top left corner of
    /// the tablet in its current logical orientation.
    ///
    /// Use `x_transformed` for transforming the axis value into a different coordinate
    /// space.
    ///
    /// If an area is defined for this device, the coordinate is in mm from the
    /// top left corner of the area. See
    /// [`Device::config_area_set_rectangle`](crate::Device::config_area_set_rectangle) for details.
    ///
    /// ## Note
    ///
    /// On some devices, returned value may be negative or larger than the width of the
    /// device. See [Out-of-bounds motion events](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-bounds)
    /// for more details.
    fn x, ffi::libinput_event_tablet_tool_get_x, f64);
    ffi_func!(
    /// Returns the Y coordinate of the tablet tool, in mm from the top left corner of
    /// the tablet in its current logical orientation.
    ///
    /// Use `y_transformed` for transforming the axis value into a different coordinate
    /// space.
    ///
    /// If an area is defined for this device, the coordinate is in mm from the
    /// top left corner of the area. See
    /// [`Device::config_area_set_rectangle`](crate::Device::config_area_set_rectangle) for details.
    ///
    /// ## Note
    ///
    /// On some devices, returned value may be negative or larger than the width of the
    /// device. See [Out-of-bounds motion events](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-bounds)
    /// for more details.
    fn y, ffi::libinput_event_tablet_tool_get_y, f64);

    /// Return the current absolute x coordinate of the tablet tool event, transformed
    /// to screen coordinates.
    ///
    /// ## Note
    ///
    /// This function may be called for a specific axis even if `x_has_changed` returns
    /// `false` for that axis. libinput always includes all device axes in the event.
    ///
    /// On some devices, returned value may be negative or larger than the width of the
    /// device. See [Out-of-bounds motion events](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-bounds)
    /// for more details.
    fn x_transformed(&self, width: u32) -> f64 {
        unsafe { ffi::libinput_event_tablet_tool_get_x_transformed(self.as_raw_mut(), width) }
    }

    /// Return the current absolute y coordinate of the tablet tool event, transformed
    /// to screen coordinates.
    ///
    /// ## Note
    ///
    /// This function may be called for a specific axis even if `y_has_changed` returns
    /// `false` for that axis. libinput always includes all device axes in the event.
    ///
    /// On some devices, returned value may be negative or larger than the width of the
    /// device. See [Out-of-bounds motion events](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-bounds)
    /// for more details.
    fn y_transformed(&self, height: u32) -> f64 {
        unsafe { ffi::libinput_event_tablet_tool_get_y_transformed(self.as_raw_mut(), height) }
    }

    /// Returns the tool that was in use during this event.
    ///
    /// ## Note
    ///
    /// Physical tool tracking requires hardware support. If unavailable, libinput
    /// creates one tool per type per tablet. See [Tracking unique tools](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-serial-numbers)
    /// for more details.
    fn tool(&self) -> TabletTool {
        unsafe {
            TabletTool::from_raw(
                ffi::libinput_event_tablet_tool_get_tool(self.as_raw_mut()),
                self.context(),
            )
        }
    }

    /// Convert into a general `TabletToolEvent` again
    fn into_tablet_tool_event(self) -> TabletToolEvent
    where
        Self: Sized,
    {
        unsafe { TabletToolEvent::from_raw(self.as_raw_mut(), self.context()) }
    }
}

impl<T: AsRaw<ffi::libinput_event_tablet_tool> + Context> TabletToolEventTrait for T {}

/// An event related to a tablet tool
#[derive(Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TabletToolEvent {
    /// One or more axes have changed state on a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// This event is only sent when the tool is in proximity, see
    /// `TabletToolProximityEvent` for details.
    ///
    /// The proximity event contains the initial state of the axis as the tool comes into
    /// proximity. An event of type `TabletToolAxisEvent` is only sent when an axis value
    /// changes from this initial state. It is possible for a tool to enter and leave
    /// proximity without sending an event of type `TabletToolAxisEvent`.
    ///
    /// An event of type `TabletToolAxisEvent` is sent when the tip state does not
    /// change. See the documentation for `TabletToolTipEvent` for more details.
    Axis(TabletToolAxisEvent),
    /// Signals that a tool has come in or out of proximity of a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// Proximity events contain each of the current values for each axis, and these
    /// values may be extracted from them in the same way they are with
    /// `TabletToolAxisEvent` events.
    ///
    /// Some tools may always be in proximity. For these tools, proximity events with
    /// `ProximityState::In` are sent only once after `DeviceAddedEvent`, and proximity
    /// events with `ProximityState::Out` are sent only once before `DeviceRemovedEvent`.
    ///
    /// If the tool that comes into proximity supports x/y coordinates, libinput
    /// guarantees that both x and y are set in the proximity event.
    ///
    /// When a tool goes out of proximity, the value of every axis should be assumed to
    /// have an undefined state and any buttons that are currently held down on the
    /// stylus are marked as released. Button release events for each button that was
    /// held down on the stylus are sent before the proximity out event.
    Proximity(TabletToolProximityEvent),
    /// Signals that a tool has come in contact with the surface of a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// On devices without distance proximity detection, the `TabletToolTipEvent` is sent
    /// immediately after `TabletToolProximityEvent` for the tip down event, and
    /// immediately before for the tip up event.
    ///
    /// The decision when a tip touches the surface is device-dependent and may be
    /// derived from pressure data or other means. If the tip state is changed by axes
    /// changing state, the `TabletToolTipEvent` includes the changed axes and no
    /// additional axis event is sent for this state change. In other words, a caller
    /// must look at both `TabletToolAxisEvent` and `TabletToolTipEvent` events to know
    /// the current state of the axes.
    ///
    /// If a button state change occurs at the same time as a tip state change, the order
    /// of events is device-dependent.
    Tip(TabletToolTipEvent),
    /// Signals that a tool has changed a logical button state on a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// Button state changes occur on their own and do not include axis state changes. If
    /// button and axis state changes occur within the same logical hardware event, the
    /// order of the `TabletToolButtonEvent` and `TabletToolAxisEvent` is device-specific.
    ///
    /// This event is not to be confused with the button events emitted by the tablet
    /// pad. See `TabletPadButtonEvent`.
    Button(TabletToolButtonEvent),
}

impl EventTrait for TabletToolEvent {
    #[doc(hidden)]
    fn as_raw_event(&self) -> *mut ffi::libinput_event {
        match self {
            TabletToolEvent::Axis(event) => event.as_raw_event(),
            TabletToolEvent::Proximity(event) => event.as_raw_event(),
            TabletToolEvent::Tip(event) => event.as_raw_event(),
            TabletToolEvent::Button(event) => event.as_raw_event(),
        }
    }
}

impl FromRaw<ffi::libinput_event_tablet_tool> for TabletToolEvent {
    unsafe fn try_from_raw(
        event: *mut ffi::libinput_event_tablet_tool,
        context: &Libinput,
    ) -> Option<Self> {
        let base = ffi::libinput_event_tablet_tool_get_base_event(event);
        match ffi::libinput_event_get_type(base) {
            ffi::libinput_event_type_LIBINPUT_EVENT_TABLET_TOOL_AXIS => Some(
                TabletToolEvent::Axis(TabletToolAxisEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_TABLET_TOOL_PROXIMITY => Some(
                TabletToolEvent::Proximity(TabletToolProximityEvent::try_from_raw(event, context)?),
            ),
            ffi::libinput_event_type_LIBINPUT_EVENT_TABLET_TOOL_TIP => Some(TabletToolEvent::Tip(
                TabletToolTipEvent::try_from_raw(event, context)?,
            )),
            ffi::libinput_event_type_LIBINPUT_EVENT_TABLET_TOOL_BUTTON => Some(
                TabletToolEvent::Button(TabletToolButtonEvent::try_from_raw(event, context)?),
            ),
            _ => None,
        }
    }
    unsafe fn from_raw(event: *mut ffi::libinput_event_tablet_tool, context: &Libinput) -> Self {
        Self::try_from_raw(event, context).expect("Unknown tablet tool event type")
    }
}

impl AsRaw<ffi::libinput_event_tablet_tool> for TabletToolEvent {
    fn as_raw(&self) -> *const ffi::libinput_event_tablet_tool {
        match self {
            TabletToolEvent::Axis(event) => event.as_raw(),
            TabletToolEvent::Proximity(event) => event.as_raw(),
            TabletToolEvent::Tip(event) => event.as_raw(),
            TabletToolEvent::Button(event) => event.as_raw(),
        }
    }
}

impl Context for TabletToolEvent {
    fn context(&self) -> &Libinput {
        match self {
            TabletToolEvent::Axis(event) => event.context(),
            TabletToolEvent::Proximity(event) => event.context(),
            TabletToolEvent::Tip(event) => event.context(),
            TabletToolEvent::Button(event) => event.context(),
        }
    }
}

ffi_event_struct! {
    /// One or more axes have changed state on a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// This event is only sent when the tool is in proximity, see
    /// `TabletToolProximityEvent` for details.
    ///
    /// The proximity event contains the initial state of the axis as the tool comes into
    /// proximity. An event of type `TabletToolAxisEvent` is only sent when an axis value
    /// changes from this initial state. It is possible for a tool to enter and leave
    /// proximity without sending an event of type `TabletToolAxisEvent`.
    ///
    /// An event of type `TabletToolAxisEvent` is sent when the tip state does not
    /// change. See the documentation for `TabletToolTipEvent` for more details.
    struct TabletToolAxisEvent, ffi::libinput_event_tablet_tool, ffi::libinput_event_tablet_tool_get_base_event
}

/// The state of proximity for a tool on a device.
///
/// The proximity of a tool is a binary state signalling whether the tool is within a
/// detectable distance of the tablet device. A tool that is out of proximity cannot
/// generate events.
///
/// On some hardware a tool goes out of proximity when it ceases to touch the surface. On /// other hardware, the tool is still detectable within a short distance (a few cm) off
/// the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProximityState {
    /// Out of proximity
    Out,
    /// In proximity
    In,
}

ffi_event_struct! {
    /// Signals that a tool has come in or out of proximity of a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// Proximity events contain each of the current values for each axis, and these
    /// values may be extracted from them in the same way they are with
    /// `TabletToolAxisEvent` events.
    ///
    /// Some tools may always be in proximity. For these tools, proximity events with
    /// `ProximityState::In` are sent only once after `DeviceAddedEvent`, and proximity
    /// events with `ProximityState::Out` are sent only once before `DeviceRemovedEvent`.
    ///
    /// If the tool that comes into proximity supports x/y coordinates, libinput
    /// guarantees that both x and y are set in the proximity event.
    ///
    /// When a tool goes out of proximity, the value of every axis should be assumed to
    /// have an undefined state and any buttons that are currently held down on the
    /// stylus are marked as released. Button release events for each button that was
    /// held down on the stylus are sent before the proximity out event.
    struct TabletToolProximityEvent, ffi::libinput_event_tablet_tool, ffi::libinput_event_tablet_tool_get_base_event
}

impl TabletToolProximityEvent {
    /// Returns the new proximity state of a tool from a proximity event.
    ///
    /// Used to check whether or not a tool came in or out of proximity during an
    /// `TabletToolProximityEvent`.
    ///
    /// See [Handling of proximity events](https://wayland.freedesktop.org/libinput/doc/latest/tablet-support.html#tablet-fake-proximity)
    /// for recommendations on proximity handling.
    pub fn proximity_state(&self) -> ProximityState {
        match unsafe { ffi::libinput_event_tablet_tool_get_proximity_state(self.as_raw_mut()) } {
            ffi::libinput_tablet_tool_proximity_state_LIBINPUT_TABLET_TOOL_PROXIMITY_STATE_OUT => {
                ProximityState::Out
            }
            ffi::libinput_tablet_tool_proximity_state_LIBINPUT_TABLET_TOOL_PROXIMITY_STATE_IN => {
                ProximityState::In
            }
            _ => panic!("libinput returned invalid 'libinput_tablet_tool_proximity_state'"),
        }
    }
}

/// The tip contact state for a tool on a device.
///
/// The tip contact state of a tool is a binary state signalling whether the tool is
/// touching the surface of the tablet device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TipState {
    /// Not touching the surface
    Up,
    /// Touching the surface
    Down,
}

ffi_event_struct! {
    /// Signals that a tool has come in contact with the surface of a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// On devices without distance proximity detection, the `TabletToolTipEvent` is sent
    /// immediately after `TabletToolProximityEvent` for the tip down event, and
    /// immediately before for the tip up event.
    ///
    /// The decision when a tip touches the surface is device-dependent and may be
    /// derived from pressure data or other means. If the tip state is changed by axes
    /// changing state, the `TabletToolTipEvent` includes the changed axes and no
    /// additional axis event is sent for this state change. In other words, a caller
    /// must look at both `TabletToolAxisEvent` and `TabletToolTipEvent` events to know
    /// the current state of the axes.
    ///
    /// If a button state change occurs at the same time as a tip state change, the order
    /// of events is device-dependent.
    struct TabletToolTipEvent, ffi::libinput_event_tablet_tool, ffi::libinput_event_tablet_tool_get_base_event
}

impl TabletToolTipEvent {
    /// Returns the new tip state of a tool from a tip event.
    ///
    /// Used to check whether or not a tool came in contact with the tablet surface or
    /// left contact with the tablet surface during an `TabletToolTipEvent`.
    pub fn tip_state(&self) -> TipState {
        match unsafe { ffi::libinput_event_tablet_tool_get_tip_state(self.as_raw_mut()) } {
            ffi::libinput_tablet_tool_tip_state_LIBINPUT_TABLET_TOOL_TIP_UP => TipState::Up,
            ffi::libinput_tablet_tool_tip_state_LIBINPUT_TABLET_TOOL_TIP_DOWN => TipState::Down,
            _ => panic!("libinput returned invalid 'libinput_tablet_tool_top_state'"),
        }
    }
}

ffi_event_struct! {
    /// Signals that a tool has changed a logical button state on a device with the
    /// `DeviceCapability::TabletTool` capability.
    ///
    /// Button state changes occur on their own and do not include axis state changes. If
    /// button and axis state changes occur within the same logical hardware event, the
    /// order of the `TabletToolButtonEvent` and `TabletToolAxisEvent` is device-specific.
    ///
    /// This event is not to be confused with the button events emitted by the tablet
    /// pad. See `TabletPadButtonEvent`.
    struct TabletToolButtonEvent, ffi::libinput_event_tablet_tool, ffi::libinput_event_tablet_tool_get_base_event
}

impl TabletToolButtonEvent {
    ffi_func!(
    /// Return the button that triggered this event.
    pub fn button, ffi::libinput_event_tablet_tool_get_button, u32);
    ffi_func!(
    /// For the button of a `TabletToolButtonEvent`, return the total number of buttons
    /// pressed on all devices on the associated seat after the the event was triggered.
    pub fn seat_button_count, ffi::libinput_event_tablet_tool_get_seat_button_count, u32);

    /// Return the button state of the event.
    pub fn button_state(&self) -> ButtonState {
        match unsafe { ffi::libinput_event_tablet_tool_get_button_state(self.as_raw_mut()) } {
            ffi::libinput_button_state_LIBINPUT_BUTTON_STATE_PRESSED => ButtonState::Pressed,
            ffi::libinput_button_state_LIBINPUT_BUTTON_STATE_RELEASED => ButtonState::Released,
            _ => panic!("libinput returned invalid 'libinput_button_state'"),
        }
    }
}
