//! Multi-finger touch session state (Orchestrator `inner.touch`); contacts held in
//! physical pixels so gesture deltas match the trackpad's libinput units.
use smithay::utils::{Physical, Point};
use compositor_support_system_input_event_base::base::Modality;

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
    /// A 2-finger swipe that began at a screen edge — swallowed by the compositor
    /// and dispatched as `(edge, angle)` (never reaches the camera or a client).
    Edge,
}

/// Which screen edge a 2-finger edge-swipe started from.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TouchEdge {
    Left,
    Right,
    Top,
    Bottom,
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

/// The sticky touch tool-mode, chosen from the touch pane and held across
/// sequences. `Touch` (default) delegates to `wl_touch`/pointer per window;
/// `Pointer` forces pointer emulation (double-tap-drag text select); `Hand`
/// makes a single finger pan anywhere; `Select` taps to append-select / drags a
/// select-box. Multi-finger camera pan/zoom stays available in every mode.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum TouchMode {
    #[default]
    Touch,
    Pointer,
    Hand,
    Select,
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
    /// Sticky tool-mode + pane visibility — set from the touch pane, held across
    /// sequences (NOT cleared by `reset`). In `Pointer` mode a finger always
    /// emulates the pointer with press-on-down, so two quick taps naturally read
    /// as a double-click and a tap-then-drag extends a text selection — the client
    /// toolkit does its own double-click detection.
    pub tool_mode: TouchMode,
    /// The world (uuid `as_u128`) the pane was summoned in, or `None` when hidden.
    /// The pane shows ONLY on that world (the per-world iced registry keeps it
    /// there), so switching worlds hides it rather than following the focus.
    pub pane_world: Option<u128>,
    /// Edge-swipe (Role::Edge) per-sequence state: the edge the swipe began at,
    /// the centroid at that moment (angle origin), and whether the binding already
    /// fired (a swipe dispatches once). Cleared by `reset`.
    pub start_edge: Option<TouchEdge>,
    pub edge_start: Point<f64, Physical>,
    pub edge_fired: bool,
    /// Pointer-mode only: the primary finger's left press is DEFERRED (not sent on
    /// down) until it's clear the touch is a click/drag and not the start of a
    /// 2-finger gesture — so a 2-finger tap fires a clean right click with no stray
    /// left click first. Sent on first motion (→ drag) or on lift (→ click);
    /// dropped silently when a second finger arrives.
    pub pending_press: bool,
    /// Pointer-mode 2-finger-tap → right-click detection: `rtap_candidate` stays
    /// true while the 2-finger phase reads as a tap (no pan, no pinch, ≤2 fingers);
    /// `rtap_ms` is when it began (duration gate) and `rtap_origin` the centroid
    /// then (the right-click location). Cleared by `reset`.
    pub rtap_candidate: bool,
    pub rtap_ms: u32,
    pub rtap_origin: Point<f64, Physical>,
    /// The device class of the most recent input event. Cross-sequence (NOT cleared
    /// by `reset`). Lets UI that differs by modality — the selection toolbar's
    /// placement, the OSK auto-summon — follow the device actually in use rather
    /// than a sticky tool-mode or the pane's visibility.
    ///
    /// One field rather than a pair of bools: the states are mutually exclusive, so
    /// a pair makes the meaningless "touch AND pen" combination representable.
    /// Mirrored into world storage as `CanvasState::input_modality` for systems that
    /// read storage instead of the tracker.
    pub modality: Modality,
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
        self.start_edge = None;
        self.edge_fired = false;
        self.pending_press = false;
        self.rtap_candidate = false;
        self.rtap_ms = 0;
    }
}

/// Classify the device class behind a raw backend event, or `None` for events that
/// say nothing about modality (keyboard, switches, device add/remove).
///
/// Lives here beside [`TouchTracker::modality`] rather than in the seat delegate so
/// the delegate stays a pure router. Pure — the caller applies the result, because
/// mirroring it into world storage needs the `Loop` this crate deliberately does not
/// depend on (that would cycle with the state root, which owns a `TouchTracker`).
pub fn classify<I: smithay::backend::input::InputBackend>(
    event: &smithay::backend::input::InputEvent<I>,
) -> Option<Modality> {
    use smithay::backend::input::{Device, DeviceCapability, Event, InputEvent::*};
    // A trackpad and a mouse are told apart by the device's GESTURE capability, not by
    // the event: a trackpad emits its physical button through the same device it emits
    // swipes and pinches through, so the button code alone cannot classify it.
    fn pointer<I: smithay::backend::input::InputBackend>(d: I::Device) -> Option<Modality> {
        Some(if d.has_capability(DeviceCapability::Gesture) {
            Modality::Trackpad
        } else {
            Modality::Mouse
        })
    }
    match event {
        TouchDown { .. } | TouchMotion { .. } | TouchUp { .. } | TouchCancel { .. }
        | TouchFrame { .. } => Some(Modality::Touch),
        TabletToolProximity { .. } | TabletToolAxis { .. } | TabletToolTip { .. }
        | TabletToolButton { .. } => Some(Modality::Pen),
        PointerMotion { event, .. } => pointer::<I>(event.device()),
        PointerMotionAbsolute { event, .. } => pointer::<I>(event.device()),
        PointerButton { event, .. } => pointer::<I>(event.device()),
        PointerAxis { event, .. } => pointer::<I>(event.device()),
        _ => None,
    }
}
