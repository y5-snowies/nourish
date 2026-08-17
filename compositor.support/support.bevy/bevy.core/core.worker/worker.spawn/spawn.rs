use compositor_model_debug_instance_record::{error, info};
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_publish_base::Board;
use compositor_support_bevy_core_shared_base::SharedContext;
use compositor_support_bevy_core_worker_base::{Job, Worker};
use std::sync::Arc;
use std::sync::mpsc::channel;

pub fn spawn(shared: &SharedContext, ctx: &Arc<WgpuVulkanContext>) -> Option<Worker> {
    let render_node = compositor_model_environment_config_base::base::get().render_node.clone();
    let board = Board::new();
    let (tx, rx) = channel::<Job>();
    let (shared, ctx, node, tx_board) = (shared.clone(), ctx.clone(), render_node.clone(), board.clone());
    match std::thread::Builder::new()
        .name("y5-bevy-worker".into())
        .spawn(move || {
            compositor_support_bevy_core_worker_serve::run(rx, tx_board, shared, ctx, node)
        }) {
        Ok(_) => {
            info!("bevy worker: thread spawned for {render_node}");
            Some(Worker::new(tx, board))
        }
        Err(e) => {
            error!("bevy worker: thread spawn failed ({e}); staying inline");
            None
        }
    }
}
