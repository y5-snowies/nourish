//! The off-thread publish → compositor wake handshake.
//!
//! Cross-thread wake for EVERY off-thread producer — the background shader
//! worker, the bevy worker and the iced worker. It lives beside the ring they
//! publish through (`publish.ring`) because it is the other half of the same
//! contract: the ring says WHICH buffer is finished, this says THAT one is.
//!
//! The compositor's redraw loop is sustained by flip -> vblank -> render, and an
//! undamaged frame queues no flip, so the loop stops. Once a producer renders off
//! the compositor thread, its publish is the ONLY event that can restart the loop
//! — hence a flag the redraw handler consults plus a ping to wake it.
//!
//! One flag and one waker for all three on purpose: the handler only needs to
//! know THAT something published, and a producer-specific flag would have to be
//! re-gated against tearing exclusivity separately.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

pub type Waker = Arc<dyn Fn() + Send + Sync>;

fn published() -> &'static AtomicBool {
    static SLOT: AtomicBool = AtomicBool::new(false);
    &SLOT
}

fn waker() -> &'static RwLock<Option<Waker>> {
    static SLOT: OnceLock<RwLock<Option<Waker>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Kernel: install the redraw ping the workers wake the compositor through.
pub fn set_offthread_waker(f: Waker) {
    *waker().write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// Worker thread: a frame is complete and published.
pub fn notify_offthread_published() {
    published().store(true, Ordering::Release);
    let w = waker().read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(f) = w {
        f();
    }
}

/// Compositor: has a producer published since we last looked? Clears the flag.
pub fn take_offthread_published() -> bool {
    published().swap(false, Ordering::Acquire)
}
