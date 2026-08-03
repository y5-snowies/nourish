//! Create the picker's bottom-right details panel as a screen-space iced surface
//! in the session world's registry (like the lock surface), and wire its
//! messages back to the compositor.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};
use std::sync::mpsc::Sender;

use compositor_monitor_compositor_iced_base::IcedHandle;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_surface_view::{PickerSurface, PickerSurfaceMessage};
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};

const PANEL_W: i32 = 300;
const PANEL_H: i32 = 150;
const MARGIN: i32 = 24;

pub fn create(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Option<IcedHandle<PickerSurface>> {
    let x = (size.w - PANEL_W - MARGIN).max(0);
    let y = (size.h - PANEL_H - MARGIN).max(0);

    let handle = compositor_y5_surface_draw_handle::handle::load(
        state,
        renderer,
        PickerSurface::new(),
        Rectangle::new(Point::from((x, y)), Size::new(PANEL_W, PANEL_H)),
        compositor_monitor_compositor_iced_base::IcedSpace::Screen,
        compositor_orchestration_draw_layer_base::base::Layer::PICKER_SCENE.bits(),
    );

    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let registry = state.inner.surface_mut().registry.as_mut()?;
    registry
        .set_message_handler(handle, move |m: &PickerSurfaceMessage| dispatch(m, &tx));
    Some(handle)
}

fn dispatch(message: &PickerSurfaceMessage, tx: &Sender<SurfaceMessage>) {
    // Forward only the actionable messages; the rest are surface-local (the
    // compositor→surface `SetWorld` and the confirm toggle).
    if matches!(
        message,
        PickerSurfaceMessage::SetWorld { .. }
            | PickerSurfaceMessage::DeleteRequest
            | PickerSurfaceMessage::DeleteCancel
    ) {
        return;
    }
    let _ = tx.send(SurfaceMessage {
        message: SurfaceMessageType::Picker(message.clone()),
    });
}
