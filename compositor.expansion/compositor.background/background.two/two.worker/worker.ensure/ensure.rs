//! Pane buffer lifecycle: allocate on first sight, reallocate on resize.

use compositor_background_two_worker_alloc::alloc;
use compositor_background_two_worker_device::device::Device;
use compositor_background_two_worker_pane::pane::{Pane, Registry};
use compositor_background_two_worker_target::target::Config;
use std::collections::HashMap;
use std::sync::Arc;

/// Make sure `key` has `slots` buffers at `size`, replacing them if either moved.
///
/// Depth is handled exactly like a resize — reallocate, do not resize in place —
/// because the published generation doubles as the ring index, so changing the
/// modulus under a live pane would rename every buffer at once.
///
/// The registry gets the new `Slots` before the old targets are dropped, so the
/// compositor never holds a handle to freed memory across the swap.
pub fn ensure(
    panes: &mut HashMap<u64, Pane>,
    registry: &Registry,
    device: &Device,
    cfg: &Config,
    key: u64,
    size: (u32, u32),
    slots: usize,
) -> Result<(), String> {
    if panes.get(&key).is_some_and(|p| p.size == size && p.targets.len() == slots) {
        return Ok(());
    }
    let targets = alloc::allocate_slots(&device.dev, &device.phd, cfg.fourcc, size, slots)?;
    // The command ring is allocated WITH the buffers and sized to match: a pane's
    // command slot IS its buffer slot, so depth is per-pane rather than shared.
    let (cmds, fences) = device.alloc_ring(slots)?;
    let pane = Pane::new(targets, cmds, fences, size);
    if let Ok(mut map) = registry.lock() {
        map.insert(key, Arc::clone(&pane.slots));
    }
    if let Some(o) = panes.insert(key, pane) {
        device.free_ring(&o.cmds, &o.fences);
        for t in o.targets { t.destroy(&device.dev); }
    }
    Ok(())
}
