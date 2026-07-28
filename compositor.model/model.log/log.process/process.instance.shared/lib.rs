//! State shared between the log drain thread and the gRPC `LogStream` server: the live
//! broadcast channel and the bounded history ring.

use std::collections::VecDeque;
use std::sync::Mutex;

use compositor_model_log_process_instance_bind::bind;

/// Live broadcast capacity (records buffered per viewer before it lags).
pub const BROADCAST_CAP: usize = 8192;
/// Recent records replayed to a viewer on connect (bounds memory; "full history" within it).
pub const HISTORY_CAP: usize = 100_000;

pub struct Shared {
    pub broadcast_tx: tokio::sync::broadcast::Sender<bind::LogRecord>,
    pub history: Mutex<VecDeque<bind::LogRecord>>,
}
