//! Pane identity: which buffer set a background draw belongs to.
//!
//! A pane key must be unique ACROSS THE WHOLE COMPOSITOR, because the worker's
//! pane map and registry are process-global while the thing being identified —
//! a viewport region — is numbered per output, restarting at 0 on every monitor.
//! Keying on the region index alone therefore made every monitor's root pane
//! collide on 0: they shared one buffer at one camera and one size, so the
//! second monitor showed the first one's view, and on a mixed-resolution desktop
//! `ensure` reallocated the whole ring twice per frame as the two sizes fought.
//!
//! The key is `output << REGION_BITS | region`, so the output it belongs to is
//! recoverable from the key — which is what lets a monitor being unplugged retire
//! exactly its own panes, with no guessing and no timeout.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Low bits reserved for the region index within one output. 65535 viewport
/// regions on one monitor is not a configuration; the clamp is only so a bad
/// index can never bleed into the output field.
pub const REGION_BITS: u32 = 16;
const REGION_MASK: u64 = (1 << REGION_BITS) - 1;

/// The output half of a key. `DefaultHasher` is unseeded, so this is stable for
/// the life of the process — which is all that is required, since keys are never
/// persisted and both sides compute them from the same `OutputKey` string.
pub fn output_prefix(output: &str) -> u64 {
    let mut h = DefaultHasher::new();
    output.hash(&mut h);
    h.finish() >> REGION_BITS
}

/// The worker-global identity of one viewport region on one output.
pub fn pane_key(output: &str, region: usize) -> u64 {
    (output_prefix(output) << REGION_BITS) | (region as u64 & REGION_MASK)
}

/// The output a key belongs to, for retiring a monitor's panes as a group.
pub fn key_output(pane: u64) -> u64 {
    pane >> REGION_BITS
}

/// The region index within that output, for retiring panes a viewport collapse
/// has left above the current region count.
pub fn key_region(pane: u64) -> usize {
    (pane & REGION_MASK) as usize
}
