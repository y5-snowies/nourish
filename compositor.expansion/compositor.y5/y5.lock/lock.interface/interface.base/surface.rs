use std::sync::mpsc::Sender;

use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Physical, Point, Rectangle, Size},
};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_lock_interface_surface::{message::LockMessage, view::LockSurface};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};
use compositor_monitor_compositor_iced_base::IcedHandle;

pub(crate) fn create(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Option<IcedHandle<LockSurface>> {
    let width = 360;
    let height = 300;
    let x = (size.w / 2 - width / 2);
    let y = (size.h / 2 - height / 2);

    let handle = compositor_y5_surface_draw_handle::handle::load(
        state,
        renderer,
        LockSurface::new(),
        Rectangle::new(Point::from((x, y)), Size::new(width, height)),
        compositor_monitor_compositor_iced_base::IcedSpace::Screen,
        compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits(),
    );

    // Clone the sender first: the registry borrow pins the same slot.
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let Some(registry) = state.inner.surface_mut().registry.as_mut() else {
        return None;
    };
    registry.set_message_handler(handle, move |message: &LockMessage| {
        lock_surface_dispatch(message, &tx);
    });

    return Some(handle);
}

fn lock_surface_dispatch(p1: &LockMessage, p2: &Sender<SurfaceMessage>) {
    match p1 {
        p1 @ compositor_y5_lock_interface_surface::message::LockMessage::Attempt { .. } => {
            info!("Sending attempt");
            p2.send(SurfaceMessage {
                message: SurfaceMessageType::LockScreen(p1.clone()),
            });
        }
        _ => {}
    }
}
