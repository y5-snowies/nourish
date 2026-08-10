//! Pane identity: which buffer set a background draw belongs to.
//!
//! A pane key must be unique ACROSS THE WHOLE COMPOSITOR, because the worker's
//! pane map and registry are process-global while the things being identified —
//! an output, a world, a viewport region — are each numbered in their own scope.
//!
//! All three fields are load bearing, and each one is a bug that was paid for:
//!
//! - **output**, because regions restart at 0 on every monitor, so every root
//!   pane collided on 0: one buffer at one camera and one size for all of them,
//!   and on mixed resolutions `ensure` reallocated the ring twice a frame as the
//!   two sizes fought over it.
//! - **world**, because two worlds shown on one output+region shared a `Pane`,
//!   hence one executor, one set of intermediate targets, one `history` image and
//!   one ping-pong pair — so a `persist` bundle sampled the other world's pixels.
//! - **region**, distinguishing a viewport slice from a full-output overlay
//!   backdrop. The picker, the lock screen and the overview each draw one, and
//!   all three would otherwise be `(output, world, region 0)`.
//!
//! Compared EXACTLY, never hashed. An earlier design packed the three into a u64
//! and hashed the world into 16 bits; world ids are persisted and rehydrated, so
//! a collision there would be deterministic and permanent for that user rather
//! than a transient glitch. Exact fields also keep `worker.serve`'s two
//! retirement axes — by output, by world — plain field compares.

use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

/// Which part of an output a pane covers.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    /// A viewport region, by index within its output. Retired when a viewport
    /// collapse leaves the index above its world's current region count.
    Viewport(u16),
    /// A full-output overlay backdrop, by namespace. It has no index, so it is
    /// never retired by a region count — only with its output or its world.
    Overlay(&'static str),
}

/// The worker-global identity of one background draw target.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PaneKey {
    /// The PLAIN `OutputKey`, never a namespaced string: `worker.serve` matches
    /// it against the key the kernel publishes when a monitor is removed, so
    /// anything decorated here is a pane that unplugging cannot reclaim.
    pub output: Arc<str>,
    /// The world whose background this draws — its provenance, which is not
    /// always the active world (an overlay draws the spawn target's).
    pub world: Uuid,
    pub region: Region,
}

impl PaneKey {
    /// One viewport region of one world on one output.
    ///
    /// Takes the output as an `Arc<str>` the CALLER already holds, so binding N
    /// regions of one output costs N refcount bumps rather than N copies of the
    /// key string. The scene hoists it once per output pass — where it was
    /// already cloning a `String` — so this allocates no more than it did before
    /// the key existed.
    pub fn viewport(output: &Arc<str>, world: Uuid, region: usize) -> Self {
        Self {
            output: Arc::clone(output),
            world,
            region: Region::Viewport(region.try_into().unwrap_or(u16::MAX)),
        }
    }

    /// A full-output overlay backdrop. `namespace` is `'static` because these
    /// are a closed set of literals — "picker", "lock", "overview".
    pub fn overlay(output: &Arc<str>, world: Uuid, namespace: &'static str) -> Self {
        Self { output: Arc::clone(output), world, region: Region::Overlay(namespace) }
    }
}

/// Readable in the worker's two log lines. These are the only diagnostics for
/// the whole class of "the wrong pane was shared", so a packed integer there was
/// exactly the wrong thing to print.
impl fmt::Display for PaneKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "output={:?} world={} region=", self.output, self.world)?;
        match &self.region {
            Region::Viewport(i) => write!(f, "{i}"),
            Region::Overlay(n) => write!(f, "{n}"),
        }
    }
}
