//! compositor.developer structured logging — **init entry point**.
//!
//! Call [`spawn`] once, as early as possible, from the compositor's entry crate
//! (`loader.main.execute`). It wires the global start instant, the runtime level mask
//! (from the `log_level` field of the central environment config), the fan-in buffer, and
//! the drain + gRPC threads. After this, any crate's level macros
//! (`error!`/`warn!`/`info!`/`trace!`) emit records.
//!
//! `environment::init()` MUST run before this (it does, as the very first line of
//! `main()`), so `get().log_level` is available here.

use compositor_model_debug_instance_record as record;

/// Capacity of the fan-in buffer (records dropped if a burst overruns the drain thread).
const BUFFER_CAP: usize = 16_384;

/// Start the developer logging process. Safe to call once; a second call only re-arms the
/// runtime level mask (the buffer/threads are installed only the first time).
pub fn spawn() {
    record::set_start(std::time::Instant::now());

    let spec = &compositor_model_environment_config_base::base::get().log_level;
    record::set_enabled_mask(record::parse_levels(spec));

    let (tx, rx) = crossbeam_channel::bounded::<record::Record>(BUFFER_CAP);
    if record::install_sender(tx) {
        compositor_model_log_process_instance::start(rx);
    }
}
