//! The log drain thread: consumes the global fan-in buffer, prints each record
//! dmesg-style (elapsed-since-start), keeps the bounded history ring, and fans records
//! out over the tokio broadcast to every connected gRPC viewer.

use std::io::Write;
use std::sync::Arc;

use compositor_model_debug_instance_record as record;
use compositor_model_log_process_instance_bind::bind;
use compositor_model_log_process_instance_shared::{HISTORY_CAP, Shared};

/// Drain the fan-in buffer: print dmesg-style, record in history, fan out to viewers.
pub fn drain(rx: crossbeam_channel::Receiver<record::Record>, shared: Arc<Shared>) {
    let stderr = std::io::stderr();
    while let Ok(rec) = rx.recv() {
        let record::Record { level, crate_name, function, message, at, ack } = rec;
        let elapsed = record::since_start(at);
        {
            // single writer (this thread) — `function` already carries the crate path
            let mut out = stderr.lock();
            let _ = writeln!(
                out,
                "[{:>5}.{:06}] {} {}: {}",
                elapsed.as_secs(),
                elapsed.subsec_micros(),
                level.label(),
                function,
                message,
            );
        }

        let proto = bind::LogRecord {
            elapsed_micros: elapsed.as_micros() as u64,
            level: level as u32,
            crate_name: crate_name.to_string(),
            function: function.to_string(),
            message,
        };

        {
            let mut hist = shared.history.lock().unwrap_or_else(|e| e.into_inner());
            if hist.len() == HISTORY_CAP {
                hist.pop_front();
            }
            hist.push_back(proto.clone());
        }
        // best-effort live fan-out; Err just means no viewers connected
        let _ = shared.broadcast_tx.send(proto);

        // abort! blocks on this — signal only after print + history + stream are done.
        if let Some(ack) = ack {
            let _ = ack.send(());
        }
    }
}
