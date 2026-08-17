//! The pointer grab that carries a toplevel along with a drag-and-drop.
//!
//! smithay's [`DnDGrab`] does the whole DnD protocol but exposes no per-motion
//! hook — `DndGrabHandler` is only `dropped`/`cancelled` — so there is nowhere
//! to reposition the dragged window from. This wraps it: every `PointerGrab`
//! method forwards straight through, and `motion` additionally queues the move.
//!
//! Bounds are written in raw smithay terms rather than `DispatchWire` on
//! purpose: `dispatch.state/state.base` constructs this grab, so this crate has
//! to sit UPSTREAM of it and cannot name `DispatchWire`.
//!
//! ## Coordinates
//!
//! `event.location` is already y5-world: `seat.pointer/pointer.input/motion.rs`
//! unprojects the hardware position through the hovered pane's camera (and the
//! shader warp) with `Transform` before handing it to smithay, and that world
//! value is what lands in `MotionEvent::location`. It is therefore the same
//! space `Space::map_element` stores, so the attach offset subtracts directly
//! and nothing here may re-apply the camera.

use smithay::input::SeatHandler;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::input::dnd::{DnDGrab, DndGrabHandler, Source};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::selection::data_device::DataDeviceHandler;

use smithay::wayland::compositor::get_parent;

use compositor_support_smithay_dispatch_wire_drag::drag::{
    Attached, ToplevelDragData, ToplevelDragHost, XdgToplevelDragV1,
};

/// Everything a state must be for this grab to exist. `WlSurface: DndFocus<D>`
/// (which `DnDGrab` needs) is provided by smithay for exactly this bound set.
pub trait DragHost:
    SeatHandler<PointerFocus = WlSurface> + DataDeviceHandler + DndGrabHandler + ToplevelDragHost
{
}
impl<D> DragHost for D where
    D: SeatHandler<PointerFocus = WlSurface> + DataDeviceHandler + DndGrabHandler + ToplevelDragHost
{
}

/// A [`DnDGrab`] that also drags a toplevel.
pub struct ToplevelDragGrab<D: DragHost + 'static, S: Source> {
    inner: DnDGrab<D, S, WlSurface>,
    /// The protocol object; consulted per motion because `attach` may land
    /// mid-drag (the detach-a-tab case attaches only once the window exists).
    drag: XdgToplevelDragV1,
}

impl<D: DragHost + 'static, S: Source> ToplevelDragGrab<D, S> {
    pub fn new(inner: DnDGrab<D, S, WlSurface>, drag: XdgToplevelDragV1) -> Self {
        Self { inner, drag }
    }

    /// The live attachment, if the drag object is still around.
    fn attached(&self) -> Option<Attached> {
        self.drag
            .data::<ToplevelDragData>()
            .filter(|_| self.drag.is_alive())
            .and_then(|d| d.live())
    }
}

/// The root of `surface`'s subsurface tree.
fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

/// Drop a focus that resolved to the dragged window itself.
///
/// The spec is explicit that "the attached window does not participate in the
/// selection of the drag target", and it has to be: the window is pinned under
/// the cursor for the whole drag, so the hit test picks it on every motion.
/// Without this the client would be offered its own drag — a browser would see
/// the tab dropped back onto the window it was just torn out of.
///
/// Compared against the subsurface ROOT, since the cursor usually lands on a
/// child surface rather than the toplevel's own.
fn suppress_self(
    focus: Option<(WlSurface, Point<f64, Logical>)>,
    attached: Option<&Attached>,
) -> Option<(WlSurface, Point<f64, Logical>)> {
    // No toplevel attached (yet) — this is an ordinary DnD, leave focus alone.
    let Some(attached) = attached else { return focus };
    focus.filter(|(surface, _)| root_of(surface) != attached.surface)
}

impl<D: DragHost + 'static, S: Source> PointerGrab<D> for ToplevelDragGrab<D, S> {
    fn motion(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        let attached = self.attached();
        if let Some(a) = &attached {
            // `event.location` is already y5-world (see the module note), so the
            // surface-local offset subtracts straight off it.
            data.queue_toplevel_drag_move(a.surface.clone(), event.location - a.offset.to_f64());
        }
        self.inner.motion(data, handle, suppress_self(focus, attached.as_ref()), event);
    }

    fn relative_motion(
        &mut self,
        data: &mut D,
        handle: &mut PointerInnerHandle<'_, D>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        self.inner.relative_motion(data, handle, focus, event);
    }

    fn button(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &ButtonEvent) {
        self.inner.button(data, handle, event);
    }

    fn axis(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, details: AxisFrame) {
        self.inner.axis(data, handle, details);
    }

    fn frame(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>) {
        self.inner.frame(data, handle);
    }

    fn gesture_swipe_begin(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GestureSwipeBeginEvent) {
        self.inner.gesture_swipe_begin(data, handle, event);
    }
    fn gesture_swipe_update(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GestureSwipeUpdateEvent) {
        self.inner.gesture_swipe_update(data, handle, event);
    }
    fn gesture_swipe_end(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GestureSwipeEndEvent) {
        self.inner.gesture_swipe_end(data, handle, event);
    }
    fn gesture_pinch_begin(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GesturePinchBeginEvent) {
        self.inner.gesture_pinch_begin(data, handle, event);
    }
    fn gesture_pinch_update(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GesturePinchUpdateEvent) {
        self.inner.gesture_pinch_update(data, handle, event);
    }
    fn gesture_pinch_end(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GesturePinchEndEvent) {
        self.inner.gesture_pinch_end(data, handle, event);
    }
    fn gesture_hold_begin(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GestureHoldBeginEvent) {
        self.inner.gesture_hold_begin(data, handle, event);
    }
    fn gesture_hold_end(&mut self, data: &mut D, handle: &mut PointerInnerHandle<'_, D>, event: &GestureHoldEndEvent) {
        self.inner.gesture_hold_end(data, handle, event);
    }

    fn start_data(&self) -> &GrabStartData<D> {
        self.inner.start_data()
    }

    fn unset(&mut self, data: &mut D) {
        // Clearing `active` here (rather than only in `dropped`/`cancelled`)
        // means the client's `destroy` stops being an `ongoing_drag` error the
        // moment the grab really ends, however it ended.
        //
        // On a physical drop this runs AFTER `dropped`: the inner `DnDGrab` passes
        // itself to `unset_grab`, so its own `unset` (and the drop) happen first,
        // and only then does the pointer unset the installed grab — this. Anything
        // that must observe the carry therefore reads it from the handler, not
        // from here.
        if let Some(d) = self.drag.data::<ToplevelDragData>().filter(|_| self.drag.is_alive()) {
            d.set_active(false);
        }
        self.inner.unset(data);
    }
}
