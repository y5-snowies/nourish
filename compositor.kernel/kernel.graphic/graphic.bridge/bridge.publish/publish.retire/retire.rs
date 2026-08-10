//! Things the compositor has RETIRED, for producers that key buffers by them.
//!
//! A mailbox rather than a direct call because the retirement is noticed in one
//! layer while the buffers live in a producer elsewhere — the same inversion the
//! publish waker solves, in the other direction.
//!
//! These are the DETERMINISTIC retirement signals. Without them a producer can
//! only infer that a thing is gone from a lapse in draw requests, which is a
//! timeout, and a timeout is wrong in both directions: short enough to reclaim
//! an unplugged 4K monitor's ring promptly is short enough to free a pane that
//! merely skipped a few frames, and then reallocate the whole thing.
//!
//! Read through the EPOCH, never by polling the list. A producer loop can run at
//! a kilohertz when idle, and taking a mutex plus cloning a `Vec` that often to
//! learn "nothing happened" would cost more than the work it guards. The epoch
//! is one relaxed atomic load.
//!
//! Two of them, because the background worker's panes are keyed by BOTH: an
//! unplugged monitor retires exactly its panes, and a world leaving the screen
//! retires exactly its panes. A pane outliving both axes is a fullscreen dmabuf
//! per slot held for nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// The retained window. A session with thousands of dock cycles or world
/// switches must not accumulate an entry per event forever; a producer further
/// behind than this is handed the whole window, which is safe because retiring
/// something whose buffers are already gone is a no-op.
const KEEP: usize = 64;

/// A bounded retirement log plus the monotonic epoch producers read it through.
pub struct Mailbox<T> {
    log: OnceLock<Mutex<Vec<T>>>,
    /// Monotonic count of retirements, ever. Also the read cursor producers hold.
    epoch: AtomicU64,
}

impl<T: Clone> Mailbox<T> {
    const fn new() -> Self {
        Self { log: OnceLock::new(), epoch: AtomicU64::new(0) }
    }

    fn log(&self) -> &Mutex<Vec<T>> {
        self.log.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// Announce: this thing is gone.
    fn retire(&self, item: T) {
        if let Ok(mut v) = self.log().lock() {
            v.push(item);
            let overflow = v.len().saturating_sub(KEEP);
            v.drain(..overflow);
            self.epoch.fetch_add(1, Ordering::Release);
        }
    }

    /// Producer: retirements ever. Cheap enough for a hot loop; compare against
    /// the cursor from the last [`Self::since`].
    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Producer: everything retired since `cursor`, and the new cursor. A cursor
    /// older than the window yields everything still held — over-reporting rather
    /// than losing a retirement, which is the direction that leaks nothing.
    fn since(&self, cursor: u64) -> (Vec<T>, u64) {
        let now = self.epoch.load(Ordering::Acquire);
        let Ok(v) = self.log().lock() else { return (Vec::new(), cursor) };
        let first = now.saturating_sub(v.len() as u64);
        let skip = cursor.max(first).saturating_sub(first) as usize;
        (v.iter().skip(skip).cloned().collect(), now)
    }
}

static OUTPUTS: Mailbox<String> = Mailbox::new();
static WORLDS: Mailbox<u128> = Mailbox::new();
static OVERLAYS: Mailbox<&'static str> = Mailbox::new();

/// Kernel: this output is gone.
pub fn retire_output(key: &str) { OUTPUTS.retire(key.to_string()); }
pub fn retired_epoch() -> u64 { OUTPUTS.epoch() }
pub fn retired_since(cursor: u64) -> (Vec<String>, u64) { OUTPUTS.since(cursor) }

/// Orchestration: this world is no longer on screen, so nothing will draw its
/// panes. Carried as a `u128` so this layer needs no uuid dependency to relay an
/// id it never interprets — both ends convert.
pub fn retire_world(id: u128) { WORLDS.retire(id); }
pub fn worlds_retired_epoch() -> u64 { WORLDS.epoch() }
pub fn worlds_retired_since(cursor: u64) -> (Vec<u128>, u64) { WORLDS.since(cursor) }

/// y5: this overlay backdrop closed. Namespace-wide and not per output, because
/// an overlay is one UI state — the picker is up or it is not, on every monitor
/// at once — so there is no per-output or per-world subtlety to get wrong.
pub fn retire_overlay(namespace: &'static str) { OVERLAYS.retire(namespace); }
pub fn overlays_retired_epoch() -> u64 { OVERLAYS.epoch() }
pub fn overlays_retired_since(cursor: u64) -> (Vec<&'static str>, u64) { OVERLAYS.since(cursor) }
