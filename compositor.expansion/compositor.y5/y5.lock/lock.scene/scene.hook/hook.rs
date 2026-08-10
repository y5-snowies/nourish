use std::time::Duration;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Scale, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::Status;
use compositor_y5_surface_protocol_base::protocol::SurfaceMessageType;

pub fn hook(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    let Status::Locked { pending, .. } = state.inner.status else {
        abort!();
    };
    // The pending→done transition is now a calloop `Timer` armed in
    // `lock_logical` (renderer-free, so it progresses even when dark). Here we only
    // drain the lock-screen auth messages once the lock is complete.
    if !pending {
        load_incoming_buffer(state, renderer, size);
    }
}

fn load_incoming_buffer(state: &mut Loop, x: &mut GlesRenderer, size: Size<i32, Physical>) {
    {
        // Drain the channel into the buffer (single slot borrow). The LOCK
        // world's own channel — the auth panel publishes into it, so the session
        // world's would be empty and every attempt silently dropped.
        let Some(surface) = compositor_y5_lock_system_base::base::surface(&mut state.inner.worlds)
        else {
            return;
        };
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
    let taken = match compositor_y5_lock_system_base::base::surface(&mut state.inner.worlds) {
        Some(surface) => std::mem::take(&mut surface.surface_message_buffer),
        None => return,
    };

    // Delegate actions
    for item in taken {
        info!("Delegate message...: {:?}", item);
        // CHECK: This omits non lock screen messages.
        match (item.message) {
            SurfaceMessageType::LockScreen(lockscreen_message) => {
                compositor_y5_lock_protocol_base::base::handle(state, x, size, lockscreen_message)
            }
            _ => {}
        }
    }
}
