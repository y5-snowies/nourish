//! Outputs the compositor has REMOVED, for producers that key buffers by output.
//!
//! A mailbox rather than a direct call because the removal is noticed in the
//! kernel while the buffers live in a producer above it — the same inversion the
//! publish waker solves, in the other direction.
//!
//! This is the DETERMINISTIC retirement signal. Without it a producer can only
//! infer that a monitor is gone from a lapse in draw requests, which is a
//! timeout, and a timeout is wrong in both directions: short enough to reclaim
//! an unplugged 4K monitor's ring promptly is short enough to free a pane that
//! merely skipped a few frames, and then reallocate the whole thing.
//!
//! Read through the EPOCH, never by polling the list. A producer loop can run at
//! a kilohertz when idle, and taking a mutex plus cloning a `Vec<String>` that
//! often to learn "nothing happened" would cost more than the work it guards.
//! The epoch is one relaxed atomic load.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

fn removed() -> &'static Mutex<Vec<String>> {
    static SLOT: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Vec::new()))
}

/// Monotonic count of removals, ever. Also the read cursor producers hold.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// The retained window. A session with thousands of dock cycles must not
/// accumulate an entry per event forever; a producer that falls further behind
/// than this is handed the whole window instead, which is safe because retiring
/// an output whose buffers are already gone is a no-op.
const KEEP: usize = 64;

/// Kernel: this output is gone.
pub fn retire_output(key: &str) {
    if let Ok(mut v) = removed().lock() {
        v.push(key.to_string());
        let overflow = v.len().saturating_sub(KEEP);
        v.drain(..overflow);
        EPOCH.fetch_add(1, Ordering::Release);
    }
}

/// Producer: how many removals have happened, ever. Cheap enough for a hot loop;
/// compare against the cursor from the last [`retired_since`].
pub fn retired_epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

/// Producer: the outputs removed since `cursor`, and the new cursor. A cursor
/// older than the window yields everything still held — over-reporting rather
/// than losing a removal, which is the direction that leaks nothing.
pub fn retired_since(cursor: u64) -> (Vec<String>, u64) {
    let now = EPOCH.load(Ordering::Acquire);
    let Ok(v) = removed().lock() else { return (Vec::new(), cursor) };
    let first = now.saturating_sub(v.len() as u64);
    let skip = cursor.max(first).saturating_sub(first) as usize;
    (v.iter().skip(skip).cloned().collect(), now)
}
