use smithay::desktop::Window;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, GrabStartData as PointerGrabStartData, MotionEvent,
    PointerGrab, PointerInnerHandle, RelativeMotionEvent,
};

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_grab_resize_motion::on_motion;
use compositor_support_smithay_state_grab_resize_surface::{ResizeEdge, ResizeSurfaceState};

pub struct GrabResize<WireObject: DispatchWire> {
    pub start_data: PointerGrabStartData<WireObject>,
    pub window: Window,
    pub edges: ResizeEdge,
    pub initial_rect: Rectangle<i32, Logical>,
    pub last_window_size: Size<i32, Logical>,
}

impl<WireObject: DispatchWire> GrabResize<WireObject> {
    pub fn start(
        start_data: PointerGrabStartData<WireObject>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
    ) -> Self {
        let initial_rect = initial_window_rect;
        ResizeSurfaceState::with(window.toplevel().unwrap().wl_surface(), |state| {
            *state = ResizeSurfaceState::Resizing { edges, initial_rect };
        });
        Self { start_data, window, edges, initial_rect, last_window_size: initial_rect.size }
    }
}

impl<WireObject: DispatchWire> PointerGrab<WireObject> for GrabResize<WireObject> {
    fn motion(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, _focus: Option<(WlSurface, Point<f64, Logical>)>, event: &MotionEvent) {
        on_motion(self.start_data.location, &self.window, self.edges, self.initial_rect, &mut self.last_window_size, data, handle, event);
    }
    fn relative_motion(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, focus: Option<(WlSurface, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
        handle.relative_motion(data, focus, event);
    }
    fn button(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &ButtonEvent) {
        handle.button(data, event);
        const BTN_LEFT: u32 = 0x110;
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
            let xdg = self.window.toplevel().unwrap();
            xdg.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Resizing);
                state.size = Some(self.last_window_size);
            });
            xdg.send_pending_configure();
            ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                *state = ResizeSurfaceState::WaitingForLastCommit { edges: self.edges, initial_rect: self.initial_rect };
            });
        }
    }
    fn axis(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, details: AxisFrame) { handle.axis(data, details) }
    fn frame(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>) { handle.frame(data); }
    fn gesture_swipe_begin(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureSwipeBeginEvent) { handle.gesture_swipe_begin(data, event) }
    fn gesture_swipe_update(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureSwipeUpdateEvent) { handle.gesture_swipe_update(data, event) }
    fn gesture_swipe_end(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureSwipeEndEvent) { handle.gesture_swipe_end(data, event) }
    fn gesture_pinch_begin(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GesturePinchBeginEvent) { handle.gesture_pinch_begin(data, event) }
    fn gesture_pinch_update(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GesturePinchUpdateEvent) { handle.gesture_pinch_update(data, event) }
    fn gesture_pinch_end(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GesturePinchEndEvent) { handle.gesture_pinch_end(data, event) }
    fn gesture_hold_begin(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureHoldBeginEvent) { handle.gesture_hold_begin(data, event) }
    fn gesture_hold_end(&mut self, data: &mut WireObject, handle: &mut PointerInnerHandle<'_, WireObject>, event: &GestureHoldEndEvent) { handle.gesture_hold_end(data, event) }
    fn start_data(&self) -> &GrabStartData<WireObject> { &self.start_data }
    fn unset(&mut self, _data: &mut WireObject) {}
}
