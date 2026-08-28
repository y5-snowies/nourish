//! `Storage`'s thread-safe sibling: token-addressed kernel data that producers on
//! their own threads can read.
//!
//! [`Storage`](compositor_support_system_storage_slot_base) holds
//! `Box<dyn Any>` behind `&mut self`, which is right for per-world state the
//! compositor thread owns exclusively — and unusable for anything an off-thread
//! producer reads. Format capability is exactly that: registered once per device
//! at boot on the compositor thread, then read from the iced, bevy and background
//! worker threads every time one allocates a buffer.
//!
//! The alternative was a bare `static` per subsystem, which is how a tree ends up
//! with four unrelated globals nobody can enumerate. This keeps ONE addressing
//! scheme — the same [`Token`] type, whose ids are already process-wide — so every
//! piece of kernel data is reached the same way whichever thread wants it.
//!
//! # Not a global
//!
//! An instance is created at boot and OWNED like any other kernel value; handles
//! are handed to the construction sites that need one. Nothing here reaches for a
//! process-wide singleton, which is the whole point of moving off bare statics.
//!
//! # Locking
//!
//! One `RwLock` over the slot vector, and values are `Sync`, so a reader takes the
//! lock only long enough to reach the value. Writes happen at boot; reads happen
//! per buffer construction, never per frame.

use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use std::any::Any;
use std::sync::RwLock;

/// Type-erased, thread-safe slot store. Slots are indexed by the process-wide
/// token id, so a lookup is an array index — no hashing.
#[derive(Default)]
pub struct StorageConcurrent {
    slots: RwLock<Vec<Option<Box<dyn Any + Send + Sync>>>>,
}

impl std::fmt::Debug for StorageConcurrent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.slots.read().map(|s| s.iter().filter(|x| x.is_some()).count()).unwrap_or(0);
        f.debug_struct("StorageConcurrent").field("occupied", &n).finish()
    }
}

impl StorageConcurrent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a slot. Panics if this storage already holds the token's slot —
    /// a slot has exactly one owner, same rule as `Storage`.
    pub fn insert<T: Any + Send + Sync>(&self, token: &'static Token<T>, value: T) {
        let id = token.ensure_id();
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        if slots.len() <= id {
            slots.resize_with(id + 1, || None);
        }
        if slots[id].is_some() {
            panic!("concurrent storage slot <{}> registered twice", std::any::type_name::<T>());
        }
        slots[id] = Some(Box::new(value));
    }

    pub fn contains<T: Any + Send + Sync>(&self, token: &'static Token<T>) -> bool {
        token
            .id()
            .and_then(|id| {
                let slots = self.slots.read().unwrap_or_else(|e| e.into_inner());
                slots.get(id).map(|s| s.is_some())
            })
            .unwrap_or(false)
    }

    /// Read a slot under the lock. The closure keeps the borrow inside the guard,
    /// which is what lets the value stay owned by the storage rather than cloned
    /// out of it on every read.
    pub fn with<T: Any + Send + Sync, R>(
        &self,
        token: &'static Token<T>,
        f: impl FnOnce(&T) -> R,
    ) -> Option<R> {
        let id = token.id()?;
        let slots = self.slots.read().unwrap_or_else(|e| e.into_inner());
        Some(f(slots.get(id)?.as_ref()?.downcast_ref::<T>()?))
    }

    /// Mutate a slot. Requires the write token, which only the owning crate can
    /// name — that visibility rule is the entire mutation policy, as in `Storage`.
    pub fn with_mut<T: Any + Send + Sync, R>(
        &self,
        token: &'static TokenMut<T>,
        f: impl FnOnce(&mut T) -> R,
    ) -> Option<R> {
        let id = token.read.id()?;
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        Some(f(slots.get_mut(id)?.as_mut()?.downcast_mut::<T>()?))
    }

    /// Insert if absent, then mutate. The registrar's slot fills itself on first
    /// use, so a caller never has to sequence "create the slot" before "register a
    /// device" — which would be one more boot-ordering rule to get wrong.
    pub fn entry<T: Any + Send + Sync + Default, R>(
        &self,
        token: &'static TokenMut<T>,
        f: impl FnOnce(&mut T) -> R,
    ) -> R {
        let id = token.read.ensure_id();
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        if slots.len() <= id {
            slots.resize_with(id + 1, || None);
        }
        let slot = slots[id].get_or_insert_with(|| Box::new(T::default()));
        f(slot.downcast_mut::<T>().expect("slot holds its own type"))
    }
}
