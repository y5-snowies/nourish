//! Multi-finger touch session state (Orchestrator `inner.touch`); contacts held in
//! physical pixels so gesture deltas match the trackpad's libinput units.
use smithay::utils::{Physical, Point};

#[derive(Clone, Copy)]
pub struct Contact {
    pub slot: i32,
    pub pos: Point<f64, Physical>,
}

/// What the whole touch sequence is doing — decided at first finger down, held
/// until every finger lifts. Client = `wl_touch` forward; Pointer = emulate the
/// pointer; Gesture = 2+ fingers → a compositor gesture.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum Role {
    #[default]
    Idle,
    Client,
    Pointer,
    Gesture,
}

/// The active compositor gesture, by finger count: 2 = pinch-zoom (+ centroid pan),
/// 3 = directional swipe, 4 = pinch to fit one / fit all.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    #[default]
    None,
    Zoom,
    Swipe,
    Fit,
}

#[derive(Default)]
pub struct TouchTracker {
    pub role: Role,
    pub mode: Mode,
    pub contacts: Vec<Contact>,
    /// Centroid at the previous motion — source of per-update pan/swipe deltas.
    pub prev_centroid: Point<f64, Physical>,
    /// Spread latched at gesture start; pinch scale = current spread / base spread.
    /// `fit_scale` is the latest, read at release for the 4-finger fit decision.
    pub base_spread: f64,
    pub fit_scale: f64,
    /// Single-finger sub-state: `pointer_glide` = glide-pan on empty canvas (vs
    /// click/drag over a window); `pointer_moved` tells a tap (→ click) from a drag.
    pub pointer_glide: bool,
    pub pointer_moved: bool,
}

impl TouchTracker {
    /// Insert or move a contact by slot.
    pub fn upsert(&mut self, slot: i32, pos: Point<f64, Physical>) {
        if let Some(c) = self.contacts.iter_mut().find(|c| c.slot == slot) {
            c.pos = pos;
        } else {
            self.contacts.push(Contact { slot, pos });
        }
    }

    /// Drop a contact; returns true if it was present.
    pub fn remove(&mut self, slot: i32) -> bool {
        let n = self.contacts.len();
        self.contacts.retain(|c| c.slot != slot);
        self.contacts.len() != n
    }

    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }
    /// Mean of all contact positions.
    pub fn centroid(&self) -> Point<f64, Physical> {
        let n = self.contacts.len().max(1) as f64;
        let (sx, sy) = self.contacts.iter().fold((0.0, 0.0), |(x, y), c| (x + c.pos.x, y + c.pos.y));
        Point::from((sx / n, sy / n))
    }

    /// Mean distance of the contacts from their centroid (the pinch "radius").
    pub fn spread(&self) -> f64 {
        let c = self.centroid();
        let n = self.contacts.len().max(1) as f64;
        self.contacts.iter().map(|k| (k.pos.x - c.x).hypot(k.pos.y - c.y)).sum::<f64>() / n
    }

    /// Clear everything back to Idle (all fingers lifted / cancelled).
    pub fn reset(&mut self) {
        self.contacts.clear();
        self.role = Role::Idle;
        self.mode = Mode::None;
        self.base_spread = 0.0;
        self.fit_scale = 1.0;
        self.pointer_glide = false;
        self.pointer_moved = false;
    }
}
