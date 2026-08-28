//! Stable element identity for elements rebuilt every frame.
//!
//! smithay keys `OutputDamageTracker`'s per-element history on [`Id`]. A fresh
//! `Id::new()` each frame therefore reads as "the old element vanished AND a new
//! one appeared": its rect is damaged twice, its opaque region never accumulates,
//! and on the DRM path the id occupying a hardware plane changes every frame.
//! Damage tracking is defeated for that element entirely.
//!
//! The counterpart trap is caching the id ALONE. A [`CommitCounter`] that never
//! moves tells the tracker the content is unchanged, so an element that animates
//! in place — same rect, different colour — is drawn once and then frozen.
//! smithay's own `SolidColorBuffer` pairs a persistent id with a counter it bumps
//! on size/colour change; this is that pairing for callers that build physical
//! rects directly rather than going through a buffer.
//!
//! Slots are addressed by `index`, which must mean the same thing every frame —
//! "the top border", not "the third rect that survived clipping". A bank is
//! per-entity: windows are distinct entities, so each window owns its own via
//! `window.user_data().get_or_insert(SolidBank::default)` and never shares slots
//! with another window. Globals (one cursor, one selection box) use a
//! `thread_local!` bank instead.
//!
//! Not `Sync` — the `RefCell` is deliberate, since a lock per element per frame
//! buys nothing on a scene assembled entirely on the compositor thread. Stored in
//! a `UserDataMap` it therefore needs `get_or_insert`, not the `_threadsafe`
//! variant, and a caller on another thread would silently get a second bank
//! rather than the existing one.

use core::cell::RefCell;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Rectangle};

struct Slot {
    id: Id,
    commit: CommitCounter,
    last: Option<(Rectangle<i32, Physical>, [f32; 4])>,
}

/// Per-entity bank of stable element identities, one per logical slot index.
#[derive(Default)]
pub struct SolidBank {
    slots: RefCell<Vec<Slot>>,
}

impl SolidBank {
    fn with_slot<T>(&self, index: usize, f: impl FnOnce(&mut Slot) -> T) -> T {
        let mut slots = self.slots.borrow_mut();
        while slots.len() <= index {
            slots.push(Slot { id: Id::new(), commit: CommitCounter::default(), last: None });
        }
        f(&mut slots[index])
    }

    /// A solid-colour element for logical slot `index`, keeping the same [`Id`]
    /// across frames and advancing the commit counter only when the rect or the
    /// colour actually changed.
    pub fn solid(&self, index: usize, rect: Rectangle<i32, Physical>, color: [f32; 4]) -> SolidColorRenderElement {
        self.with_slot(index, |slot| {
            if slot.last != Some((rect, color)) {
                slot.last = Some((rect, color));
                slot.commit.increment();
            }
            SolidColorRenderElement::new(slot.id.clone(), rect, slot.commit, color, Kind::Unspecified)
        })
    }

    /// Identity for slot `index` whose CONTENT changes every frame — a texture
    /// re-imported per frame, say, where this type cannot see what changed. The
    /// id is stable (so the tracker keeps its history) but the counter advances
    /// unconditionally, preserving full per-frame damage. Prefer [`Self::solid`]
    /// whenever the inputs are comparable.
    pub fn content(&self, index: usize) -> (Id, CommitCounter) {
        self.with_slot(index, |slot| {
            slot.commit.increment();
            (slot.id.clone(), slot.commit)
        })
    }
}
