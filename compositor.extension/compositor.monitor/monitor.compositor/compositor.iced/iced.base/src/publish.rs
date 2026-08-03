//! The board the iced worker publishes onto and the compositor reads.
//!
//! One entry per live instance: the dmabuf whose write has COMPLETED, plus the
//! generation and the size it was rendered at. The compositor builds its render
//! element from this and nothing else — it never touches an `IcedRuntime`, the
//! shared `Renderer`, or the slot being drawn into.
//!
//! Separate from the bevy board of the same shape on purpose: this one is keyed
//! by iced's `HandleId`, and making iced depend on a bevy crate to share ~40
//! lines would invert the layering for no gain.

use crate::handle::HandleId;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::utils::{Physical, Point, Size};
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct Published {
    pub dmabuf: Dmabuf,
    pub generation: u64,
    /// The size actually rendered, which may lag a resize the compositor has
    /// already applied. Drawing at the requested size would sample past it.
    pub size: Size<i32, Physical>,
    /// The WORLD location that was current when this size was requested.
    ///
    /// Travels with the size so the pair is atomic. Position and size change
    /// together and in opposite directions when a left or top edge is dragged;
    /// taking the position from now and the size from a frame ago makes the rect
    /// disagree with itself, and the edge the user is NOT holding walks by
    /// however far the drag moved in that latency. Carried in WORLD space, not
    /// screen: the camera is applied at projection, so zoom and pan stay
    /// immediate while only the resize pair lags.
    pub location: Point<i32, Physical>,
}

#[derive(Clone, Default)]
pub struct Board {
    inner: Arc<RwLock<HashMap<HandleId, Published>>>,
    /// Any instance dirty, mid-animation, or holding an unretired publish. This
    /// is what keeps the compositor's redraw loop alive: iced rasterizes only
    /// when something changed, so without it a frame produced off-thread would
    /// have no scheduled frame to become visible in.
    wants: Arc<AtomicBool>,
    /// Read-only copies of UI state the compositor inspects synchronously. Only
    /// the UIs that opted into `IcedSnapshot` appear here; see that trait for why
    /// one frame of staleness is the right trade.
    snaps: Arc<RwLock<HashMap<HandleId, Arc<dyn Any + Send + Sync>>>>,
}

impl Board {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, id: HandleId, entry: Published) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).insert(id, entry);
    }

    /// `None` before the first finished frame; the instance draws nothing then,
    /// rather than showing an unwritten buffer.
    pub fn get(&self, id: HandleId) -> Option<Published> {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
    }

    pub fn remove(&self, id: HandleId) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).remove(&id);
        self.snaps.write().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }

    pub fn set_wants(&self, yes: bool) {
        self.wants.store(yes, Ordering::Release);
    }

    pub fn wants(&self) -> bool {
        self.wants.load(Ordering::Acquire)
    }

    pub fn put_snapshot(&self, id: HandleId, snap: Arc<dyn Any + Send + Sync>) {
        self.snaps.write().unwrap_or_else(|e| e.into_inner()).insert(id, snap);
    }

    pub fn snapshot(&self, id: HandleId) -> Option<Arc<dyn Any + Send + Sync>> {
        self.snaps.read().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
    }
}
