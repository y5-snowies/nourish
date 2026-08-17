use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};
use compositor_model_debug_instance_level::{Level, Record};

/// Fan-in buffer sender. Installed by `compositor_model_log_process_main::spawn`.
pub static SENDER: OnceLock<crossbeam_channel::Sender<Record>> = OnceLock::new();

/// Application start, for dmesg-style relative timestamps.
pub static START: OnceLock<Instant> = OnceLock::new();

/// Runtime-enabled level mask (bit per `Level`).
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// Install the global fan-in sender. Returns false if already installed.
pub fn install_sender(tx: crossbeam_channel::Sender<Record>) -> bool {
    SENDER.set(tx).is_ok()
}

/// Record the application start instant (call once, as early as possible).
pub fn set_start(now: Instant) {
    let _ = START.set(now);
}

/// Elapsed time of a record since application start (0 if start is unset).
pub fn since_start(at: Instant) -> Duration {
    match START.get() {
        Some(start) => at.saturating_duration_since(*start),
        None => Duration::ZERO,
    }
}

/// Set the runtime-enabled level mask.
pub fn set_enabled_mask(mask: u8) {
    ENABLED.store(mask, Ordering::Relaxed);
}

/// Runtime gate: is `level` currently enabled?
#[inline]
pub fn runtime_enabled(level: Level) -> bool {
    ENABLED.load(Ordering::Relaxed) & (1 << (level as u8)) != 0
}

/// Non-blocking push ("ping" the buffer). Silently drops if the buffer is full or the log
/// process has not been started.
#[inline]
pub fn push(record: Record) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.try_send(record);
    }
}

/// Backing function for `abort!`: log the message at Error level and **block until the
/// drain thread has fully handled it** (printed + streamed) before panicking. The wait is
/// bounded so a dead/absent drain never hangs the abort. Always logs regardless of the
/// level features / `COMPOSITOR_LOG_LEVEL` — an abort is fatal.
#[cold]
#[inline(never)]
pub fn abort(crate_name: &'static str, function: &'static str, message: String) -> ! {
    if let Some(tx) = SENDER.get() {
        let (ack_tx, ack_rx) = crossbeam_channel::bounded::<()>(1);
        let record = Record {
            level: Level::Error,
            crate_name,
            function,
            message: message.clone(),
            at: Instant::now(),
            ack: Some(ack_tx),
        };
        if tx.try_send(record).is_ok() {
            // Wait for the drain's signal; bounded so abort can't hang on a stalled drain.
            let _ = ack_rx.recv_timeout(Duration::from_secs(1));
        }
    }
    panic!("{message}");
}
