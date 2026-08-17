use std::marker::PhantomData;
use compositor_support_bevy_core_scene_base::BevyScene;

/// Opaque, type-erased instance identifier. Stable for the lifetime of the
/// instance; destroying and recreating returns a new id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandleId(pub u64);

impl HandleId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for HandleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bevy#{}", self.0)
    }
}

/// Typed handle. Carries the scene type as a phantom so operations requiring
/// `S::Command` are checked at compile time.
///
/// Drop semantics: dropping the handle does NOT destroy the instance. Call
/// `BevyRegistry::destroy` explicitly to release resources.
pub struct BevyHandle<S: BevyScene> {
    pub id: HandleId,
    _marker: PhantomData<fn() -> S>,
}

impl<S: BevyScene> BevyHandle<S> {
    #[doc(hidden)]
    pub fn new(id: HandleId) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// Strip the type tag.
    pub fn untyped(self) -> HandleId {
        self.id
    }
}

impl<S: BevyScene> Clone for BevyHandle<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: BevyScene> Copy for BevyHandle<S> {}

impl<S: BevyScene> PartialEq for BevyHandle<S> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<S: BevyScene> Eq for BevyHandle<S> {}

impl<S: BevyScene> std::hash::Hash for BevyHandle<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl<S: BevyScene> std::fmt::Debug for BevyHandle<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BevyHandle<{}>({})", std::any::type_name::<S>(), self.id)
    }
}
