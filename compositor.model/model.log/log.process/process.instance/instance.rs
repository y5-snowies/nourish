use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use compositor_model_debug_instance_record as record;
use compositor_model_log_process_instance_shared::{BROADCAST_CAP, HISTORY_CAP, Shared};
pub use compositor_model_log_process_instance_bind::{SOCKET, bind};

/// Start the drain + gRPC threads. Call exactly once.
pub fn start(rx: crossbeam_channel::Receiver<record::Record>) {
    let (broadcast_tx, _seed) = tokio::sync::broadcast::channel::<bind::LogRecord>(BROADCAST_CAP);
    let shared = Arc::new(Shared {
        broadcast_tx,
        history: Mutex::new(VecDeque::with_capacity(HISTORY_CAP)),
    });

    {
        let shared = shared.clone();
        let _ = std::thread::Builder::new()
            .name("y5-log-drain".into())
            .spawn(move || compositor_model_log_process_instance_drain::drain(rx, shared));
    }
    let _ = std::thread::Builder::new()
        .name("y5-log-grpc".into())
        .spawn(move || compositor_model_log_process_instance_serve::serve(shared));
}
