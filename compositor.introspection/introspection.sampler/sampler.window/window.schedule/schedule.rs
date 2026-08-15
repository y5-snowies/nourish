//! Sampler queue entries, cadence constants, and timing functions.
//!
//! Each window is visited within `T(N)` seconds, where N is the number
//! of registered placeholders: N ≤ 10 → T = 30s (hard floor); N > 10 →
//! T = 30 + 30·(1 − e^{−N/30}) → asymptotic 60s. Per tick, up to
//! [`BATCH_CAP`] entries are processed from the FRONT of a FIFO queue
//! and pushed to the back; tick interval is `T / ceil(N / BATCH_CAP)`.

use std::collections::VecDeque;
use std::time::Duration;

use uuid::Uuid;
use compositor_introspection_extraction_window_base::{Meta, MetaNode};

/// A registration message: main thread → sampler thread.
pub enum Registration {
    Add(Entry),
    Remove(Uuid),
    /// Sample these windows on the very next tick, ignoring the cadence.
    ///
    /// Each item carries the WAYLAND half of that window's identity, read on the
    /// main thread (`extract_surface_meta`) because this thread cannot: app_id
    /// and title live on the surface, not in `/proc`. A window that isn't
    /// registered yet is registered by this.
    Refresh(Vec<(Uuid, Meta)>),
}

/// One registered placeholder in the sampling queue.
pub struct Entry {
    pub uuid: Uuid,
    pub pid: u32,
    pub previous_meta: MetaNode,
}

/// Maximum samples per tick. Caps spike size.
pub const BATCH_CAP: usize = 10;

/// Below FLOOR_N placeholders, T(N) is hard-floored to FLOOR_T.
pub const FLOOR_N: usize = 10;
pub const FLOOR_T_SECS: f32 = 30.0;

/// T(N) asymptote.
pub const CEIL_T_SECS: f32 = 60.0;

/// Exponential decay constant in the T(N) formula.
pub const DECAY_K: f32 = 30.0;

/// Quick debounce: applied to the first flush after a quiet period.
/// Keeps the perceived sampling cycle near T (instead of T + SLOW_DEBOUNCE).
pub const QUICK_DEBOUNCE: Duration = Duration::from_secs(1);

/// Slow debounce: applied while in steady-state sampling. Caps flush
/// rate so the main thread isn't woken too often.
pub const SLOW_DEBOUNCE: Duration = Duration::from_secs(10);

pub fn target_full_pass(n: usize) -> Duration {
    if n <= FLOOR_N {
        return Duration::from_secs_f32(FLOOR_T_SECS);
    }
    let n_f = n as f32;
    let t = FLOOR_T_SECS + (CEIL_T_SECS - FLOOR_T_SECS) * (1.0 - (-n_f / DECAY_K).exp());
    Duration::from_secs_f32(t.clamp(FLOOR_T_SECS, CEIL_T_SECS))
}

pub fn tick_interval(n: usize) -> Duration {
    let ticks_per_pass = n.div_ceil(BATCH_CAP).max(1);
    target_full_pass(n) / ticks_per_pass as u32
}

/// New registrations push to the front of the queue, so they get
/// sampled on the very next tick.
///
/// Returns how many entries this message asks to be sampled IMMEDIATELY — non-zero
/// only for [`Registration::Refresh`], which is a user-visible request (the
/// overview opening) rather than the background cadence. The caller uses it to
/// bring the next tick forward and to widen that one tick past [`BATCH_CAP`], so
/// a refresh of every window on the world lands in a single pass.
pub fn apply_registration(queue: &mut VecDeque<Entry>, reg: Registration) -> usize {
    match reg {
        Registration::Add(e) => {
            queue.retain(|q| q.uuid != e.uuid);
            queue.push_front(e);
            0
        }
        Registration::Remove(uuid) => {
            queue.retain(|q| q.uuid != uuid);
            0
        }
        Registration::Refresh(items) => {
            let mut forced = 0;
            for (uuid, surface) in items {
                let Some(pid) = surface.pid else { continue };
                let held = queue.iter().position(|q| q.uuid == uuid).and_then(|i| queue.remove(i));
                // Never registered (mapped without credentials, say) — the
                // refresh is also its registration.
                let mut entry = held
                    .unwrap_or_else(|| Entry { uuid, pid, previous_meta: MetaNode::leaf(surface.clone()) });
                entry.pid = pid;
                merge_surface(&mut entry.previous_meta.meta, surface);
                queue.push_front(entry);
                forced += 1;
            }
            forced
        }
    }
}

/// Overwrite the wayland-derived fields of a captured `Meta` with a freshly read
/// surface snapshot, leaving every `/proc`-derived field alone. The mirror of
/// what `refresh_meta_from_pid` preserves — that call keeps these across a
/// re-extraction precisely because this thread cannot re-read them.
fn merge_surface(into: &mut Meta, surface: Meta) {
    into.app_id = surface.app_id;
    into.title = surface.title;
    into.pid = surface.pid;
    into.uid = surface.uid;
    into.gid = surface.gid;
}
