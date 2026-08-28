//! What the negotiation decided, and what it cost — the observability half.
//!
//! Absorbs the former `bridge.negotiate/negotiate.report`. Both facts it records
//! are decided on hot paths (an allocation happens per ring slot per surface per
//! resize; a node lookup runs whenever a producer starts) while the DECISION is a
//! property of the machine and the configuration — the same answer every time. So
//! each distinct answer is logged once and repeats are dropped, which turns a
//! would-be flood into a short readable block at startup.

use compositor_kernel_graphic_format_rule_base::rule;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Fourcc, Modifier};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn seen() -> &'static Mutex<HashSet<String>> {
    static SLOT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// True the first time this exact line would be emitted. Public because the
/// same latch is what keeps `format.resolve`'s per-answer block from repeating:
/// an answer is a property of the machine, so a repeat carries no information.
pub fn first(key: String) -> bool {
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
    let class = rule::label(rule::classify(chosen));
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

/// Every `(fourcc, modifier)` pair in a set, grouped by fourcc, one line each.
///
/// Counts alone cannot answer the question these logs exist for — "is the pair I
/// need actually here" — so the pairs themselves are printed. Bounded: this runs
/// at registration and at construction, never per frame.
pub fn dump(indent: &str, set: &FormatSet) {
    let mut by: Vec<(u32, Vec<Modifier>)> = Vec::new();
    for f in set.iter() {
        let code = f.code as u32;
        match by.iter_mut().find(|(c, _)| *c == code) {
            Some((_, v)) => v.push(f.modifier),
            None => by.push((code, vec![f.modifier])),
        }
    }
    by.sort_by_key(|(c, _)| *c);
    for (code, mut mods) in by {
        mods.sort_by_key(|m| std::cmp::Reverse(rule::rank(*m)));
        let name = Fourcc::try_from(code)
            .map(|f| format!("{f:?}"))
            .unwrap_or_else(|_| format!("{code:#x}"));
        info!("{indent}{name} ({}): {}", mods.len(), rule::describe_all(&mods));
    }
}

/// What an intersection COST, per fourcc.
///
/// The gap this closes: a fourcc dropped entirely is already visible, because the
/// advertisement logs the withheld list. A fourcc that SURVIVES while losing
/// modifiers is not visible at all — the 14→7 narrowing on this machine had to be
/// caught by hand with `dmabuf-probe`. Both are reported here, once per distinct
/// outcome, so a narrowing announces itself.
pub fn narrowing(what: &str, before: &FormatSet, after: &FormatSet) {
    // Keyed by the numeric fourcc: `DrmFourcc` is Hash + Eq but not Ord, and the
    // report wants a stable order, so sort the numbers at the end rather than
    // relying on a map that cannot be ordered.
    let count = |s: &FormatSet| {
        let mut m = std::collections::HashMap::<u32, usize>::new();
        for f in s.iter() {
            *m.entry(f.code as u32).or_default() += 1;
        }
        m
    };
    let (b, a) = (count(before), count(after));
    let name = |c: u32| Fourcc::try_from(c).map(|f| format!("{f:?}")).unwrap_or_else(|_| format!("{c:#x}"));
    let mut dropped: Vec<u32> = b.keys().filter(|c| !a.contains_key(*c)).copied().collect();
    dropped.sort_unstable();
    let mut thinned: Vec<(u32, usize, usize)> = a
        .iter()
        .filter_map(|(c, n)| b.get(c).filter(|was| *was > n).map(|was| (*c, *was, *n)))
        .collect();
    thinned.sort_unstable();
    if dropped.is_empty() && thinned.is_empty() {
        return;
    }
    if !first(format!("narrow:{what}:{}:{}", dropped.len(), thinned.len())) {
        return;
    }
    if !dropped.is_empty() {
        let names: Vec<String> = dropped.iter().map(|c| name(*c)).collect();
        info!(
            "format narrowing ({what}): {} fourcc(s) dropped entirely: {}",
            dropped.len(), names.join(", ")
        );
    }
    if !thinned.is_empty() {
        let detail: Vec<String> =
            thinned.iter().map(|(c, was, now)| format!("{} {was}->{now}", name(*c))).collect();
        info!(
            "format narrowing ({what}): {} fourcc(s) survived with FEWER modifiers: {}",
            thinned.len(),
            detail.join(", ")
        );
    }
}

/// An answer was produced under a non-`Ok` outcome. Latched per distinct message.
pub fn outcome(what: &str, outcome: rule::Outcome) {
    match outcome {
        rule::Outcome::Ok => {}
        rule::Outcome::Degraded(why) => {
            if first(format!("degraded:{what}:{why}")) {
                warn!(
                    "format {what}: DEGRADED — {why}. A constraint was dropped because nothing \
                     had published it, so this answer is wider than the truth."
                );
            }
        }
        rule::Outcome::Refused(why) => {
            if first(format!("refused:{what}:{why}")) {
                warn!(
                    "format {what}: REFUSED — {why}. Every term was present and they share \
                     nothing, so there is no safe buffer to allocate."
                );
            }
        }
    }
}
