use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_surface_protocol_base::protocol::SurfaceMessageType;

pub fn hook(state: &mut Loop, x: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Nothing is constructed here. The shared iced GPU context and every world's
    // `IcedRegistry` are pre-created at startup: the loader block-waits the async
    // wgpu init, stores the context in the kernel, then prewarms each world's
    // registry (`surface_system::base::ensure_registry`) — off the render path.
    // This hook only drains the surface-message buffer into the (asserted-present)
    // registry of the focused world.
    load_incoming_buffer(state, x, size);
}

fn load_incoming_buffer(state: &mut Loop, x: &mut GlesRenderer, size: Size<i32, Physical>) {
    {
        // Drain the channel into the buffer (single slot borrow).
        let surface = state.inner.surface_mut();
        'drain: while true {
            if let Ok(ok) = surface.surface_message_buffer_channel.1.try_recv() {
                info!("Buffer item receive");
                surface.surface_message_buffer.push(ok);
            } else {
                break 'drain;
            }
        }
    }

    // Takes the buffer by draining it
    let taken = std::mem::take(&mut state.inner.surface_mut().surface_message_buffer);

    // Delegate actions
    for item in taken {
        info!("Delegate message...: {:?}", item);
        match (item.message) {
            SurfaceMessageType::Placeholder(placeholder_message) => {
                compositor_y5_placeholder_interface_base::handler::delegate(
                    state,
                    placeholder_message,
                );
            }
            SurfaceMessageType::Launcher(launcher_message) => {
                compositor_y5_launcher_protocol_interface::interface::handle(
                    state,
                    x,
                    launcher_message,
                )
            }
            SurfaceMessageType::Group(group_message) => {
                compositor_y5_group_interface_base::protocol::handle(state, x, group_message)
            }
            SurfaceMessageType::Capture(capture_message) => {
                compositor_y5_graphic_capture_interface::interface::handle(state, x, capture_message)
            }
            SurfaceMessageType::Selection(selection_forward) => {
                compositor_y5_select_overlay_interface::interface::handle(state, x, selection_forward)
            }
            SurfaceMessageType::Overview(overview_message) => {
                compositor_y5_overview_interface_base::base::handle(state, x, overview_message)
            }
            SurfaceMessageType::TouchPane(touch_pane_message) => {
                compositor_y5_touch_pane_handle::handle::delegate(state, touch_pane_message)
            }
            SurfaceMessageType::Guide(guide_message) => {
                compositor_y5_guide_interface_handle::handle::delegate(state, guide_message)
            }
            SurfaceMessageType::Shader(shader_message) => {
                compositor_y5_guide_shader_handle::handle::delegate(state, x, shader_message)
            }
            SurfaceMessageType::Osk(osk_message) => {
                compositor_y5_osk_board_handle::handle::delegate(state, osk_message)
            }
            SurfaceMessageType::Settings(settings_message) => {
                compositor_configurator_settings_interface_handle::handle::handle(state, x, settings_message)
            }
            _ => {} // SurfaceMessageType::LockScreen(lockscreen_message) => {
                    //     compositor_y5_lock_protocol_base::base::handle(state, x, size, lockscreen_message)
                    // }
        }
    }
}
