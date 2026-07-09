//! A synthetic `InputBackend` that lets touch reuse the whole pointer pipeline.
//!
//! Touchscreen parity (for now) mirrors the trackpad by driving the pointer: the
//! primary finger emulates absolute motion + a left button. Rather than duplicate
//! the pointer routing (overview / viewport / world-bus / native press), we
//! synthesize pointer events on this fake backend and feed them through the very
//! same generic `motion::absolute::<I>` / `button::button::<I>` entry points the
//! real pointer uses — so touch and mouse stay in lockstep.
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
    DeviceCapability, Event, GestureBeginEvent, GestureEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, InputBackend, PointerAxisEvent,
    PointerButtonEvent, PointerMotionAbsoluteEvent, UnusedEvent,
};
use std::path::PathBuf;

/// BTN_LEFT from `<linux/input-event-codes.h>`; a tap emulates a left click.
pub const BTN_LEFT: u32 = 0x110;

/// Zero-variant marker backend. Never instantiated; it exists only to carry the
/// associated event types below so the generic pointer handlers can be reused.
pub enum TouchEmu {}

/// Synthetic device reported by every emulated event. The pointer routing never
/// inspects it, but the `Event` trait requires a device.
#[derive(PartialEq, Eq, Hash)]
pub struct EmuDevice;

impl Device for EmuDevice {
    fn id(&self) -> String {
        "touch-emulated-pointer".into()
    }
    fn name(&self) -> String {
        "touch-emulated-pointer".into()
    }
    fn has_capability(&self, c: DeviceCapability) -> bool {
        c == DeviceCapability::Pointer
    }
    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }
    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

/// Absolute-motion event carrying the touch point as a normalized 0..1 fraction
/// of the output — exactly how the real backends express it, so every consumer's
/// `position_transformed(size)` scales correctly regardless of the size passed.
pub struct AbsEvent {
    pub time: u32,
    pub nx: f64,
    pub ny: f64,
}

impl Event<TouchEmu> for AbsEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000
    }
    fn device(&self) -> EmuDevice {
        EmuDevice
    }
}
impl AbsolutePositionEvent<TouchEmu> for AbsEvent {
    fn x(&self) -> f64 {
        self.nx
    }
    fn y(&self) -> f64 {
        self.ny
    }
    fn x_transformed(&self, width: i32) -> f64 {
        self.nx * width as f64
    }
    fn y_transformed(&self, height: i32) -> f64 {
        self.ny * height as f64
    }
}
impl PointerMotionAbsoluteEvent<TouchEmu> for AbsEvent {}

/// Button event: a touch down/up emulates a BTN_LEFT press/release.
pub struct BtnEvent {
    pub time: u32,
    pub state: ButtonState,
}
impl Event<TouchEmu> for BtnEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000
    }
    fn device(&self) -> EmuDevice {
        EmuDevice
    }
}
impl PointerButtonEvent<TouchEmu> for BtnEvent {
    fn button_code(&self) -> u32 {
        BTN_LEFT
    }
    fn state(&self) -> ButtonState {
        self.state
    }
}

/// Scroll event synthesized from a two-finger pan. Reported as a `Finger` source
/// so the axis handler treats it as a canvas pan (not a discrete wheel/zoom).
pub struct AxisEvent {
    pub time: u32,
    pub horizontal: f64,
    pub vertical: f64,
}
impl Event<TouchEmu> for AxisEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000
    }
    fn device(&self) -> EmuDevice {
        EmuDevice
    }
}
impl PointerAxisEvent<TouchEmu> for AxisEvent {
    fn amount(&self, axis: Axis) -> Option<f64> {
        Some(match axis {
            Axis::Horizontal => self.horizontal,
            Axis::Vertical => self.vertical,
        })
    }
    fn amount_v120(&self, _axis: Axis) -> Option<f64> {
        None
    }
    fn source(&self) -> AxisSource {
        AxisSource::Finger
    }
    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

/// One synthetic pinch event covering begin/update/end. `scale` is absolute
/// relative to begin (== 1.0 at begin), matching libinput's convention so the
/// existing `pinch::` handlers reuse unchanged; `dx`/`dy` are the centroid delta.
pub struct PinchEvent {
    pub time: u32,
    pub fingers: u32,
    pub scale: f64,
    pub dx: f64,
    pub dy: f64,
    pub cancelled: bool,
}
impl Event<TouchEmu> for PinchEvent {
    fn time(&self) -> u64 {
        self.time as u64 * 1000
    }
    fn device(&self) -> EmuDevice {
        EmuDevice
    }
}
impl GestureBeginEvent<TouchEmu> for PinchEvent {
    fn fingers(&self) -> u32 {
        self.fingers
    }
}
impl GestureEndEvent<TouchEmu> for PinchEvent {
    fn cancelled(&self) -> bool {
        self.cancelled
    }
}
impl GesturePinchBeginEvent<TouchEmu> for PinchEvent {}
impl GesturePinchEndEvent<TouchEmu> for PinchEvent {}
impl GesturePinchUpdateEvent<TouchEmu> for PinchEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }
    fn delta_y(&self) -> f64 {
        self.dy
    }
    fn scale(&self) -> f64 {
        self.scale
    }
    fn rotation(&self) -> f64 {
        0.0
    }
}

impl InputBackend for TouchEmu {
    type Device = EmuDevice;
    type PointerMotionAbsoluteEvent = AbsEvent;
    type PointerButtonEvent = BtnEvent;
    type PointerAxisEvent = AxisEvent;
    type GesturePinchBeginEvent = PinchEvent;
    type GesturePinchUpdateEvent = PinchEvent;
    type GesturePinchEndEvent = PinchEvent;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = ();
}
