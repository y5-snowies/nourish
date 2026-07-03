//! Viewport tree: a root `Viewport` of `Slot`s (+ `floating` panes); each slot owns a `Camera`.
use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_y5_camera_state_base::state::Camera;
use smithay::utils::{Physical, Rectangle};

/// Stable per-world slot identity (the shortcut target; also seeds damage Ids).
pub type SlotId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Vertical,
    Horizontal,
}

/// An array of slots, or a floating pane wrapping a (splittable) viewport.
pub enum Viewport {
    Slots { axis: Axis, slots: Vec<Slot> },
    Floating { rect: Rectangle<i32, Physical>, inner: Box<Viewport> },
}

/// A drawing cell. `camera` is live for a leaf (`content` None); `weight` is its
/// size share within the parent `Slots` array (separator-drag adjusts it).
pub struct Slot {
    pub id: SlotId,
    pub camera: Camera,
    pub content: Option<Box<Viewport>>,
    pub weight: f64,
}

pub struct Viewports {
    pub root: Viewport,
    /// Detached panes overlaid on `root` (drawn on top); each a `Floating`.
    pub floating: Vec<Viewport>,
    /// Keyboard-shortcut target (split/detach) — set by clicking a pane.
    pub active: SlotId,
    /// Pane under the cursor — operative for all pointer input.
    pub pointer: SlotId,
    pub next_id: SlotId,
    /// Windows visible per leaf slot (refreshed each render, transient) — drives per-window fractional scale.
    pub visible: std::collections::HashMap<SlotId, Vec<uuid::Uuid>>,
}

/// An in-progress viewport separator drag (resize): the two adjacent slots and the
/// physical geometry captured at drag start, so motion redistributes their `weight`s
/// without re-deriving from a moving layout. The split moves; `sum_weight` is preserved.
#[derive(Clone, Copy)]
pub struct SeparatorDrag {
    pub a: SlotId,
    pub b: SlotId,
    pub axis: Axis,
    /// Physical cursor coord along the divide axis at drag start.
    pub start_along: f64,
    /// Physical lengths of slots a and b along the axis at drag start.
    pub a_len: f64,
    pub b_len: f64,
    /// `weight(a) + weight(b)` at drag start (preserved; only the split moves).
    pub sum_weight: f64,
}

/// An in-progress floating-pane drag: move (`Super`-drag near an edge) or resize
/// (`Super+Shift`-drag near an edge). `index` is the floating pane; the edge bools
/// fix which sides move on resize.
#[derive(Clone, Copy)]
pub struct FloatingDrag {
    pub index: usize,
    pub resize: bool,
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub start_cursor: (f64, f64),
    pub start_rect: Rectangle<i32, Physical>,
}

/// Per-MONITOR view state: one independent [`Viewports`] per physical output, keyed
/// by the output's EDID key. Each monitor is its OWN viewport — its own camera
/// (pan/zoom) and its own split/float panes — NOT extended and NOT mirrored. Always
/// holds the bootstrap entry under `String::new()` (the sole / not-yet-identified
/// output) so single-output behavior is byte-identical: the sole output uses that
/// one `Viewports`, exactly as the pre-multi-output token did.
pub struct OutputViews {
    pub map: std::collections::HashMap<String, Viewports>,
    /// The output the direct-storage readers (systems holding only world storage,
    /// not the Orchestrator) operate on — kept in sync with the cursor's output by
    /// the pointer path. The Orchestrator's own accessors resolve by render/cursor
    /// output directly and don't depend on this.
    pub current: String,
    /// In-progress viewport separator drag (resize), if any. TRANSIENT (an active
    /// cursor drag) — like `Viewports.visible`, it is NOT persisted (`ViewportsRecord`
    /// omits it). Single, not per-output: a drag is globally singular (one cursor →
    /// one drag) and always on `current`. The interaction logic lives in
    /// `viewport.interaction`; the pointer input path drives it.
    pub separator_drag: Option<SeparatorDrag>,
    /// In-progress floating-pane move/resize drag, if any. Transient; see
    /// [`separator_drag`](Self::separator_drag).
    pub floating_drag: Option<FloatingDrag>,
}

impl Default for OutputViews {
    fn default() -> Self {
        let mut map = std::collections::HashMap::new();
        map.insert(String::new(), Viewports::default());
        OutputViews { map, current: String::new(), separator_drag: None, floating_drag: None }
    }
}

impl OutputViews {
    /// Resolve to a key that EXISTS in the map: `want` if present, else `current`,
    /// else the bootstrap (always present). Never returns a missing key.
    fn resolved(&self, want: &str) -> String {
        if self.map.contains_key(want) {
            want.to_string()
        } else if self.map.contains_key(&self.current) {
            self.current.clone()
        } else {
            self.map.keys().next().expect("bootstrap entry always present").clone()
        }
    }
    /// This output's view tree (its own camera + panes). Falls back to `current` /
    /// bootstrap when `key` is unknown.
    pub fn views(&self, key: &str) -> &Viewports {
        let k = self.resolved(key);
        self.map.get(&k).expect("resolved above")
    }
    pub fn views_mut(&mut self, key: &str) -> &mut Viewports {
        let k = self.resolved(key);
        self.map.get_mut(&k).expect("resolved above")
    }
    /// The current output's view tree (for the direct-storage readers).
    pub fn current_views(&self) -> &Viewports {
        self.views(&self.current)
    }
    pub fn current_views_mut(&mut self) -> &mut Viewports {
        let c = self.current.clone();
        self.views_mut(&c)
    }
    /// Ensure `key` has its own view tree, creating it if new — WITHOUT changing
    /// `current`. The render loop calls this per drawn output so each monitor has its
    /// own camera, while leaving `current` (the input systems' target) on the
    /// cursor's output. On the FIRST real output the bootstrap "" tree's state (the
    /// single-output camera + splits, incl. what was restored from disk) is carried
    /// over rather than reset, so a pre-multi-output saved layout isn't lost.
    pub fn ensure(&mut self, key: &str) {
        if !key.is_empty()
            && !self.map.contains_key(key)
            && self.map.len() == 1
            && self.map.contains_key("")
        {
            let vp = self.map.remove("").expect("checked present");
            self.map.insert(key.to_string(), vp);
        }
        self.map.entry(key.to_string()).or_default();
    }

    /// Ensure `key`'s view tree AND make it current — the output the input systems
    /// (pan/zoom, hit-test) operate on. Called from the pointer path for the cursor's
    /// output.
    pub fn set_current(&mut self, key: &str) {
        self.ensure(key);
        self.current = key.to_string();
    }
}

pub static OUTPUT_VIEWS: Token<OutputViews> = Token::new();
pub static OUTPUT_VIEWS_MUT: TokenMut<OutputViews> = TokenMut::new(&OUTPUT_VIEWS);

impl Default for Viewports {
    fn default() -> Self {
        let slot = Slot { id: 0, camera: Camera::default(), content: None, weight: 1.0 };
        let root = Viewport::Slots { axis: Axis::Vertical, slots: vec![slot] };
        Viewports { root, floating: Vec::new(), active: 0, pointer: 0, next_id: 1, visible: std::collections::HashMap::new() }
    }
}

impl Viewport {
    /// Depth-first search for slot `id` (matches container slots too).
    pub fn find(&self, id: SlotId) -> Option<&Slot> {
        match self {
            Viewport::Slots { slots, .. } => slots.iter().find_map(|s| if s.id == id { Some(s) } else { s.content.as_ref().and_then(|v| v.find(id)) }),
            Viewport::Floating { inner, .. } => inner.find(id),
        }
    }
    pub fn find_mut(&mut self, id: SlotId) -> Option<&mut Slot> {
        match self {
            Viewport::Slots { slots, .. } => slots.iter_mut().find_map(|s| if s.id == id { Some(s) } else { s.content.as_mut().and_then(|v| v.find_mut(id)) }),
            Viewport::Floating { inner, .. } => inner.find_mut(id),
        }
    }
    /// First leaf in document order (the always-present fallback).
    pub fn first_leaf(&self) -> &Slot {
        match self {
            Viewport::Slots { slots, .. } => match &slots[0].content { Some(inner) => inner.first_leaf(), None => &slots[0] },
            Viewport::Floating { inner, .. } => inner.first_leaf(),
        }
    }
}

impl Viewports {
    fn panes(&self) -> impl Iterator<Item = &Viewport> {
        std::iter::once(&self.root).chain(self.floating.iter())
    }
    /// Camera of the slot with `id`, searched across the root AND floating panes.
    pub fn camera_of(&self, id: SlotId) -> Option<&Camera> {
        self.panes().find_map(|v| v.find(id)).map(|s| &s.camera)
    }
    pub fn camera_of_mut(&mut self, id: SlotId) -> Option<&mut Camera> {
        if self.root.find(id).is_some() {
            return self.root.find_mut(id).map(|s| &mut s.camera);
        }
        self.floating.iter_mut().find_map(|v| v.find_mut(id)).map(|s| &mut s.camera)
    }
    /// Operative camera (pane under the cursor, `pointer`; first leaf if stale).
    pub fn focus_camera(&self) -> &Camera {
        self.camera_of(self.pointer).unwrap_or_else(|| &self.root.first_leaf().camera)
    }
    pub fn focus_camera_mut(&mut self) -> &mut Camera {
        let id = if self.camera_of(self.pointer).is_some() { self.pointer } else { self.root.first_leaf().id };
        self.camera_of_mut(id).expect("first_leaf always resolves")
    }
}
