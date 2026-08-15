//! Sampler queue entries, cadence constants, and timing functions.
//!
//! Each window is visited within `T(N)` seconds, where N is the number
//! of registered placeholders: N ≤ 10 → T = 30s (hard floor); N > 10 →
//! T = 30 + 30·(1 − e^{−N/30}) → asymptotic 60s. Per tick, up to
//! [`BATCH_CAP`] entries are processed from the FRONT of a FIFO queue
//! and pushed to the back; tick interval is `T / ceil(N / BATCH_CAP)`.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use uuid::Uuid;
use compositor_introspection_extraction_window_base::{Meta, MetaNode};

/// A registration message: main thread → sampler thread.
pub enum Registration {
    Add(Entry),
    Remove(Uuid),
    /// Sample these windows on the next tick if they are STALE, ignoring the
    /// cadence. Anything sampled within [`IMMEDIATE_MIN_AGE`] is dropped from the
    /// request, so a request where everything is fresh costs nothing and does not
    /// disturb the queue order.
    ///
    /// Each item carries the WAYLAND half of that window's identity, read on the
    /// main thread (`extract_surface_meta`) because this thread cannot: app_id
    /// and title live on the surface, not in `/proc`. A window that isn't
    /// registered yet is registered by this.
    RequestImmediate(Vec<(Uuid, Meta)>),
    /// The windows worth sampling at all — the current world's. Entries outside
    /// it stay queued (with their captured meta) but are skipped, so re-entering
    /// their world resumes them without a fresh process-tree walk.
    Scope(HashSet<Uuid>),
}

/// One registered placeholder in the sampling queue.
pub struct Entry {
    pub uuid: Uuid,
    pub pid: u32,
    pub previous_meta: MetaNode,
    /// When this entry was last sampled. `None` = never, which is always stale.
    pub last_sampled: Option<Instant>,
}

impl Entry {
    /// A fresh registration: never sampled, so an immediate request will take it.
    pub fn new(uuid: Uuid, pid: u32, previous_meta: MetaNode) -> Self {
        Self { uuid, pid, previous_meta, last_sampled: None }
    }

    /// Whether an immediate request should act on this entry.
    pub fn stale(&self, now: Instant) -> bool {
        self.last_sampled.is_none_or(|at| now.duration_since(at) >= IMMEDIATE_MIN_AGE)
    }
}

/// Maximum samples per tick. Caps spike size.
pub const BATCH_CAP: usize = 10;

/// How old a sample must be before an immediate request will re-take it.
/// Matched to the cadence floor: within one full pass the entry was going to be
/// visited anyway, so re-taking it is pure duplicate work — which is what makes
/// repeatedly opening the overview free.
pub const IMMEDIATE_MIN_AGE: Duration = Duration::from_secs(FLOOR_T_SECS as u64);

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
/// Returns how many entries this message asks to be sampled IMMEDIATELY —
/// non-zero only for [`Registration::RequestImmediate`], and only for the ones
/// that were actually stale. The caller uses it to bring the next tick forward
/// and to widen that one tick past [`BATCH_CAP`], so a request covering every
/// window on a world lands in a single pass. Zero means there is nothing to do
/// and the queue was left alone.
pub fn apply_registration(
    queue: &mut VecDeque<Entry>,
    scope: &mut Option<HashSet<Uuid>>,
    reg: Registration,
) -> usize {
    match reg {
        Registration::Add(e) => {
            // A window that just mapped belongs to the world in view, so it joins
            // the scope — otherwise it would be skipped until the next switch.
            if let Some(scope) = scope.as_mut() {
                scope.insert(e.uuid);
            }
            queue.retain(|q| q.uuid != e.uuid);
            queue.push_front(e);
            0
        }
        Registration::Remove(uuid) => {
            queue.retain(|q| q.uuid != uuid);
            if let Some(scope) = scope.as_mut() {
                scope.remove(&uuid);
            }
            0
        }
        Registration::Scope(ids) => {
            *scope = Some(ids);
            0
        }
        Registration::RequestImmediate(items) => {
            let now = Instant::now();
            let mut forced = 0;
            for (uuid, surface) in items {
                let Some(pid) = surface.pid else { continue };
                let index = queue.iter().position(|q| q.uuid == uuid);
                // Fresh enough that the cadence has it covered. Checked BEFORE
                // removing, so a fresh entry keeps its place in the rotation.
                if index.is_some_and(|i| !queue[i].stale(now)) {
                    continue;
                }
                // Never registered (mapped without credentials, say) — the
                // request is also its registration.
                let held = index.and_then(|i| queue.remove(i));
                let mut entry = held.unwrap_or_else(|| Entry::new(uuid, pid, MetaNode::leaf(surface.clone())));
                entry.pid = pid;
                merge_surface(&mut entry.previous_meta.meta, surface);
                if let Some(scope) = scope.as_mut() {
                    scope.insert(uuid);
                }
                queue.push_front(entry);
                forced += 1;
            }
            forced
        }
    }
}

/// Whether to spend a `/proc` pass on this entry.
///
/// Two ways to qualify:
///
/// - It is in SCOPE — the current world's windows. No scope at all (before the
///   first world switch) means everything is.
/// - It has NEVER been sampled. A fresh window is always taken, whatever world
///   it belongs to: until its first sample its record holds only what the
///   synchronous map-time extraction found, and leaving that unfilled until its
///   world happens to be focused is how a placeholder ends up permanently blank.
///   It costs one pass, once, and thereafter the scope governs.
pub fn should_sample(scope: &Option<HashSet<Uuid>>, entry: &Entry) -> bool {
    entry.last_sampled.is_none() || scope.as_ref().is_none_or(|s| s.contains(&entry.uuid))
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
