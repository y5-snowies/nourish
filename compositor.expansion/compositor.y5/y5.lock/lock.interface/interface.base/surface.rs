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

    // Built in the LOCK world's own registry, not `surface_mut()`'s — see
    // `lock_system_base::surface`. Created directly rather than through
    // `handle::load`, which resolves the spawn target; its draw-order `register`
    // is a no-op here anyway, because a `Screen`-space surface keeps its own band
    // and never interleaves with windows.
    let gpu = state.inner.environment.GPU.clone();
    // Clone the sender first: the registry borrow pins the same slot.
    let tx = {
        let Some(surface) = compositor_y5_lock_system_base::base::surface(&mut state.inner.worlds)
        else {
            return None;
        };
        surface.surface_message_buffer_channel.0.clone()
    };
    let Some(registry) = compositor_y5_lock_system_base::base::registry(&mut state.inner.worlds)
    else {
        return None;
    };
    let handle = registry
        .create_in_space(
            gpu.as_str(),
            LockSurface::new(),
            renderer,
            Point::from((x, y)),
            Size::new(width, height),
            compositor_monitor_compositor_iced_base::IcedSpace::Screen,
            compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits(),
        )
        .ok()?;
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
