//! Multi-finger touch session state (Orchestrator `inner.touch`). Contacts are
//! held in physical pixels so gesture deltas match the trackpad's libinput units.
use smithay::utils::{Physical, Point};

/// One active finger.
#[derive(Clone, Copy)]
pub struct Contact {
    pub slot: i32,
    pub pos: Point<f64, Physical>,
}

/// What the whole touch sequence is doing — decided at the first finger down and
/// held until every finger lifts.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum Role {
    #[default]
    Idle,
    /// First finger landed on a client surface → forward to `wl_touch`.
    Client,
    /// One finger on the desktop / compositor UI → emulate the pointer.
    Pointer,
    /// Two+ fingers on the desktop → a compositor gesture.
    Gesture,
}

/// The active compositor gesture, chosen by finger count within `Gesture`.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    #[default]
    None,
    /// 2 fingers: pinch-zoom (with centroid pan).
    Zoom,
    /// 3 fingers: directional swipe → navigator.
    Swipe,
    /// 4 fingers: pinch to fit one / fit all.
    Fit,
}

#[derive(Default)]
pub struct TouchTracker {
    pub role: Role,
    pub mode: Mode,
    pub contacts: Vec<Contact>,
    /// Centroid at the previous motion — source of per-update pan/swipe deltas.
    pub prev_centroid: Point<f64, Physical>,
    /// Spread (mean distance to centroid) latched at gesture start; pinch scale =
    /// current spread / base spread. `fit_scale` is the latest, read at release
    /// for the 4-finger fit decision.
    pub base_spread: f64,
    pub fit_scale: f64,
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
    }
}
