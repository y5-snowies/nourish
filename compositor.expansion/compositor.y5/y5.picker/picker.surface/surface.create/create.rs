//! Create the picker's bottom-right details panel as a screen-space iced surface
//! in the PICKER world's own registry (like the lock surface), and wire its
//! messages back to the compositor through that world's own channel.
//!
//! Nothing here touches `surface_mut()` / `camera()` / the spawn target: the panel
//! belongs to the picker, which MOVES the spawn target when a cell is entered.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};
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
    let gpu = state.inner.environment.GPU.clone();

    // Screen-space, so no `DrawOrder` registration is owed (that is world-space
    // only) — which is the whole of what `surface.draw/draw.handle::load` adds
    // over `create_screen`, and it resolves the session registry. So: direct.
    let surface = compositor_y5_picker_system_base::base::surface(&mut state.inner.worlds)?;
    let tx = surface.surface_message_buffer_channel.0.clone();
    let registry = surface.registry.as_mut()?;
    let handle = registry
        .create_screen(
            &gpu.as_str(),
            PickerSurface::new(),
            renderer,
            Point::from((x, y)),
            Size::new(PANEL_W, PANEL_H),
            compositor_orchestration_draw_layer_base::base::Layer::PICKER_SCENE.bits(),
        )
        .ok()?;
    registry.set_message_handler(handle, move |m: &PickerSurfaceMessage| dispatch(m, &tx));
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
