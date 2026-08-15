use compositor_support_bevy_core_handle_base::HandleId;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::utils::{Physical, Size};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct Published {
    /// The finished buffer. Never the slot the worker is drawing into.
    pub dmabuf: Dmabuf,
    /// Bumped on every publish. The element's commit counter follows it, so a
    /// frame that published nothing reports no damage.
    pub generation: u64,
    /// The size the worker actually rendered at, which may lag a resize the
    /// compositor has already asked for. Drawing at the requested size while the
    /// buffer is still the old one would stretch it for a frame.
    pub size: Size<i32, Physical>,
}

#[derive(Clone, Default)]
pub struct Board {
    inner: Arc<RwLock<HashMap<HandleId, Published>>>,
}

impl Board {
    pub fn new() -> Self {
        Self::default()
    }

    /// Worker: a frame is complete and safe to sample.
    pub fn publish(&self, id: HandleId, entry: Published) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).insert(id, entry);
    }

    /// Compositor: the newest finished frame for `id`. `None` before the first
    /// publish — the instance draws nothing until then, rather than showing an
    /// unwritten buffer.
    pub fn get(&self, id: HandleId) -> Option<Published> {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
    }

    /// Drop an instance's entry, so its dmabuf refcount falls with the instance
    /// rather than lingering until the whole board is dropped.
    pub fn remove(&self, id: HandleId) {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }
}
