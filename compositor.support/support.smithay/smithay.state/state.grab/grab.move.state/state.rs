use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, GrabStartData as PointerGrabStartData, MotionEvent,
    PointerGrab, PointerInnerHandle, RelativeMotionEvent,
};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;

/// `GrabMovement` represents the state of the compositor when a user is dragging a window.
///
/// In Smithay, a "Grab" intercepts standard input routing. When active, the compositor
/// hijacks pointer events and processes them here.
pub struct GrabMovement<WireObject: DispatchWire> {
    pub start_data: PointerGrabStartData<WireObject>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl<WireObject: DispatchWire> PointerGrab<WireObject> for GrabMovement<WireObject> {
    fn motion(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, _focus: Option<(WlSurface, Point<f64, Logical>)>, event: &MotionEvent) {
        handle.motion(data, None, event);
        let delta = event.location - self.start_data.location;
        let _new_location = self.initial_window_location.to_f64() + delta;
    }
    fn relative_motion(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, focus: Option<(WlSurface, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
        handle.relative_motion(data, focus, event);
    }
    fn button(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &ButtonEvent) {
        handle.button(data, event);
        const BTN_LEFT: u32 = 0x110;
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }
    fn axis(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, details: AxisFrame) {
        handle.axis(data, details)
    }
    fn frame(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>) {
        handle.frame(data);
    }
    fn gesture_swipe_begin(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureSwipeBeginEvent) { handle.gesture_swipe_begin(data, event) }
    fn gesture_swipe_update(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureSwipeUpdateEvent) { handle.gesture_swipe_update(data, event) }
    fn gesture_swipe_end(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureSwipeEndEvent) { handle.gesture_swipe_end(data, event) }
    fn gesture_pinch_begin(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GesturePinchBeginEvent) { handle.gesture_pinch_begin(data, event) }
    fn gesture_pinch_update(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GesturePinchUpdateEvent) { handle.gesture_pinch_update(data, event) }
    fn gesture_pinch_end(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GesturePinchEndEvent) { handle.gesture_pinch_end(data, event) }
    fn gesture_hold_begin(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureHoldBeginEvent) { handle.gesture_hold_begin(data, event) }
    fn gesture_hold_end(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureHoldEndEvent) { handle.gesture_hold_end(data, event) }
    fn start_data(&self) -> &GrabStartData<WireObject> {
        &self.start_data
    }
    fn unset(&mut self, _data: &mut WireObject) {}
}
