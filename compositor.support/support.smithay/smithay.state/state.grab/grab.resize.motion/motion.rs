use smithay::desktop::Window;
use smithay::input::pointer::{MotionEvent, PointerInnerHandle};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::compositor;
use smithay::wayland::shell::xdg::SurfaceCachedState;
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_grab_resize_surface::ResizeEdge;

#[allow(clippy::too_many_arguments)]
pub fn on_motion<WireObject: DispatchWire>(
    start_location: Point<f64, Logical>,
    window: &Window,
    edges: ResizeEdge,
    initial_rect: Rectangle<i32, Logical>,
    last_window_size: &mut Size<i32, Logical>,
    data: &mut WireObject,
    handle: &mut PointerInnerHandle<'_, WireObject>,
    event: &MotionEvent,
) {
    handle.motion(data, None, event);

    let mut delta = event.location - start_location;
    let mut new_window_width = initial_rect.size.w;
    let mut new_window_height = initial_rect.size.h;

    if edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
        if edges.intersects(ResizeEdge::LEFT) { delta.x = -delta.x; }
        new_window_width = (initial_rect.size.w as f64 + delta.x) as i32;
    }
    if edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
        if edges.intersects(ResizeEdge::TOP) { delta.y = -delta.y; }
        new_window_height = (initial_rect.size.h as f64 + delta.y) as i32;
    }

    let (min_size, max_size) =
        compositor::with_states(window.toplevel().unwrap().wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let data = guard.current();
            (data.min_size, data.max_size)
        });

    let min_width = min_size.w.max(1);
    let min_height = min_size.h.max(1);
    let max_width = if max_size.w == 0 { i32::MAX } else { max_size.w };
    let max_height = if max_size.h == 0 { i32::MAX } else { max_size.h };

    *last_window_size = Size::from((
        new_window_width.max(min_width).min(max_width),
        new_window_height.max(min_height).min(max_height),
    ));

    let xdg = window.toplevel().unwrap();
    xdg.with_pending_state(|state| {
        state.states.set(xdg_toplevel::State::Resizing);
        state.size = Some(*last_window_size);
    });
    xdg.send_pending_configure();
}
