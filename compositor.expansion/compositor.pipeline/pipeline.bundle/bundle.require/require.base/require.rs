//! What a bundle asks the engine for — one entry per engine cost.
//!
//! This list is the ONLY thing that makes the engine do extra work for a bundle.
//! Declare nothing and a multipass bundle costs exactly what a single-pass shader
//! costs: no world set collected, no client buffer exported, no descriptor-indexing
//! feature enabled, no offscreen image allocated.
//!
//! Keeping that true is worth more than the convenience of INFERRING a requirement
//! from use, which is why sampling `content` without declaring `composited_scene`
//! is an error rather than a quiet allocation. A cost that appears because of how
//! a shader happens to be written is a cost nobody can find again later — and the
//! whole reason this type exists is that the feature had four of them.
//!
//! One entry per COST, not per binding. `window_geometry` and `world_geometry`
//! bind the same UBO and differ only in membership, but they are separate entries
//! because the whole-band set holds a reference to every world drawable's buffer
//! for the frame and the window-only set does not.

use serde::Deserialize;

/// One engine resource a pass requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    /// Geometry UBO at `@group(1) @binding(0)`, client windows only.
    WindowGeometry,
    /// Geometry UBO, every world drawable in draw order, each kind-tagged.
    WorldGeometry,
    /// Bindless texture array at `@group(1) @binding(1)`, client windows only.
    /// Needs the device's descriptor-indexing feature.
    WindowTextures,
    /// Bindless texture array, every world drawable. Needs descriptor indexing.
    WorldTextures,
    /// The `content` image — the scene composited so far. After-content only.
    CompositedScene,
    /// The `windows` image — client windows over transparency, background excluded.
    WindowLayer,
    /// The `history` image — the previous frame's composited scene.
    PreviousFrame,
    /// The `Times` UBO at `@group(1) @binding(2)`: per-entry timestamps for when
    /// each window opened, entered, left, took focus, became topmost and was
    /// resized. Index-aligned with the geometry, so it implies the world set.
    WindowTimes,
    /// The `Pointer` UBO at `@group(1) @binding(3)`: cursor position, held
    /// buttons and the moments they last changed. Seat-wide, so unlike every
    /// other entry here it is one value rather than one per drawable — and it
    /// implies no world set.
    PointerState,
}

impl Requirement {
    const fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// The built-in target name a pass samples to reach this resource, for the
    /// three that are images. `None` for the two bindings, which have no name in
    /// `inputs` — a pass reaches them through `@group(1)` instead.
    pub const fn target(self) -> Option<&'static str> {
        match self {
            Requirement::CompositedScene => Some("content"),
            Requirement::WindowLayer => Some("windows"),
            Requirement::PreviousFrame => Some("history"),
            _ => None,
        }
    }

    /// Every entry, for a reader that has to enumerate them (the settings panel
    /// listing what a bundle asked for). Kept beside the enum so a new entry that
    /// forgets to appear here is one edit away rather than one search away.
    pub const ALL: [Requirement; 9] = [
        Requirement::WindowGeometry,
        Requirement::WorldGeometry,
        Requirement::WindowTextures,
        Requirement::WorldTextures,
        Requirement::CompositedScene,
        Requirement::WindowLayer,
        Requirement::PreviousFrame,
        Requirement::WindowTimes,
        Requirement::PointerState,
    ];

    /// What declaring this entry costs the engine, in one line, for the user.
    ///
    /// Beside the entry rather than in the panel that shows it: the whole premise
    /// of this type is that one declaration equals one cost, and a description of
    /// the cost that lives somewhere else is free to drift away from the gate it
    /// describes.
    pub const fn cost(self) -> &'static str {
        match self {
            Requirement::WindowGeometry => "collects the window set each frame",
            Requirement::WorldGeometry => "collects the whole world band each frame",
            Requirement::WindowTextures => "binds window textures; needs descriptor indexing",
            Requirement::WorldTextures => "binds every world texture; needs descriptor indexing",
            Requirement::CompositedScene => "composites the scene offscreen first",
            Requirement::WindowLayer => "composites a separate window layer",
            Requirement::PreviousFrame => "keeps and copies the previous frame",
            Requirement::WindowTimes => "tracks and uploads per-window timestamps",
            Requirement::PointerState => "uploads the cursor position and buttons",
        }
    }

    /// The requirement a built-in target name implies, so `plan()` can name the
    /// entry a bundle forgot rather than just refusing the input.
    pub fn of_target(name: &str) -> Option<Requirement> {
        match name {
            "content" => Some(Requirement::CompositedScene),
            "windows" => Some(Requirement::WindowLayer),
            "history" => Some(Requirement::PreviousFrame),
            _ => None,
        }
    }
}

/// Spelled as the bundle spells it. Every message about a requirement names the
/// string an author can paste back into `pipeline.json` — `WorldTextures` sends
/// them looking for something that is not in the file.
impl std::fmt::Display for Requirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Requirement::WindowGeometry => "window_geometry",
            Requirement::WorldGeometry => "world_geometry",
            Requirement::WindowTextures => "window_textures",
            Requirement::WorldTextures => "world_textures",
            Requirement::CompositedScene => "composited_scene",
            Requirement::WindowLayer => "window_layer",
            Requirement::PreviousFrame => "previous_frame",
            Requirement::WindowTimes => "window_times",
            Requirement::PointerState => "pointer_state",
        })
    }
}

const GEOMETRY: u16 = Requirement::WindowGeometry.bit() | Requirement::WorldGeometry.bit();
/// Timestamps are per-ENTRY, so they are meaningless without the entries they are
/// aligned with: declaring them collects the set exactly as a geometry entry does.
const TIMES: u16 = Requirement::WindowTimes.bit();
const TEXTURES: u16 = Requirement::WindowTextures.bit() | Requirement::WorldTextures.bit();
const WHOLE: u16 = Requirement::WorldGeometry.bit() | Requirement::WorldTextures.bit();

/// The union of what a bundle requires. `Default`/empty is the load-bearing case:
/// with no bundle loaded every gate reads this and finds nothing to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requires(u16);

impl Requires {
    pub const NONE: Requires = Requires(0);

    pub fn of(items: impl IntoIterator<Item = Requirement>) -> Self {
        Requires(items.into_iter().fold(0, |a, r| a | r.bit()))
    }

    pub const fn has(self, r: Requirement) -> bool {
        self.0 & r.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn union(self, other: Self) -> Self {
        Requires(self.0 | other.0)
    }

    /// The world set must be collected, published and kept alive for this frame.
    /// Both interfaces imply it — the textures are index-aligned with the rects.
    pub const fn world_set(self) -> bool {
        self.0 & (GEOMETRY | TEXTURES | TIMES) != 0
    }

    /// The per-window timestamps must be tracked, gathered and uploaded. Off, the
    /// engine does none of it and the `Times` UBO is left as it was.
    pub const fn times(self) -> bool {
        self.0 & TIMES != 0
    }

    /// The seat-wide pointer block must be uploaded.
    ///
    /// Deliberately NOT part of `world_set()`: the pointer is one value that has
    /// nothing to do with which drawables are on screen, so a bundle that only
    /// wants the cursor pays for no collection at all.
    pub const fn pointer(self) -> bool {
        self.0 & Requirement::PointerState.bit() != 0
    }

    /// The bindless array is bound, so the device needs descriptor indexing and
    /// every owned drawable's pixels must be reachable by a second device.
    pub const fn textures(self) -> bool {
        self.0 & TEXTURES != 0
    }

    /// Membership is the whole world band rather than client windows only.
    pub const fn whole_band(self) -> bool {
        self.0 & WHOLE != 0
    }

    /// Any of the three offscreen images, i.e. the `ContentPath` must exist.
    pub const fn offscreen(self) -> bool {
        self.has(Requirement::CompositedScene)
            || self.has(Requirement::WindowLayer)
            || self.has(Requirement::PreviousFrame)
    }

    /// Raw bits, for the process-wide device gates: those live in the kernel,
    /// which cannot name this type without depending on the background layer.
    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Self {
        Requires(bits)
    }
}
