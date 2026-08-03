//! One place that answers "which GPU, and which modifier, for what".
//!
//! Both facts were previously spread across the sites that decide them, phrased
//! differently, and several were not logged at all — notably the IMPLICIT
//! allocation path, which is exactly the case a reader needs to see, because it
//! is what a failed negotiation silently degrades into.
//!
//! # Why these latch
//!
//! Both are decided on a hot path: an allocation happens per ring slot per
//! surface per resize, and a node lookup runs whenever a producer starts. The
//! DECISION, though, is a property of the machine and the configuration — it is
//! the same answer every time. So each distinct answer is logged once and
//! repeats are dropped, which turns a would-be flood into a short, readable
//! block at startup that says what the compositor picked and why.

use compositor_kernel_graphic_bridge_negotiate_classify::classify;
use smithay::backend::allocator::{Fourcc, Modifier};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::OnceLock;

fn seen() -> &'static Mutex<HashSet<String>> {
    static SLOT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// True the first time this exact line would be emitted.
fn first(key: String) -> bool {
    seen().lock().map(|mut s| s.insert(key)).unwrap_or(true)
}

/// A GPU node was chosen for something. `purpose` says what for — "composite",
/// "scanout", "background worker", "bevy wgpu" — so a multi-GPU machine's log
/// reads as a list of jobs and the device each one landed on.
pub fn node(purpose: &str, node: &str) {
    if first(format!("node:{purpose}")) {
        info!("gpu node: {purpose} -> {node}");
    }
}

/// A buffer was allocated. `chosen` is what the driver ACTUALLY picked, not what
/// was asked for, and `candidates` is how much choice it had — `0` meaning the
/// implicit path, where the modifier is whatever gbm decided and can come back
/// `INVALID`.
pub fn allocation(purpose: &str, node: &str, fourcc: Fourcc, candidates: usize, chosen: Modifier) {
    let class = classify::label(classify::classify(chosen));
    if !first(format!("alloc:{purpose}:{fourcc:?}:{chosen:?}")) {
        return;
    }
    match candidates {
        0 => info!(
            "gpu alloc: {purpose} on {node} -> {fourcc:?} {class} ({chosen:?}) via the IMPLICIT \
             path — no modifier list was offered, so the driver chose alone"
        ),
        n => info!(
            "gpu alloc: {purpose} on {node} -> {fourcc:?} {class} ({chosen:?}), negotiated from \
             {n} candidate(s)"
        ),
    }
}
