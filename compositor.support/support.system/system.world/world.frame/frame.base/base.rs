use std::any::Any;
use std::time::Duration;

/// Draw layer: lower draws first (further back). Bands are spaced so systems
/// can slot between them without renumbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Layer(pub u16);

pub const BACKGROUND: Layer = Layer(0);
// wlr-layer-shell Background/Bottom sit BELOW window/world content, matching the
// hit-test order (interface.core/hit.rs): a Background wallpaper or Bottom panel
// only receives a click where nothing above it is hit. Splitting the four
// layer-shell layers into their own bands (instead of one lumped LAYERSHELL band
// above everything) is what keeps render z-order consistent with hit-testing —
// e.g. an interactive wallpaper draws under windows AND is clicked only through them.
pub const LAYER_BACKGROUND: Layer = Layer(10);
pub const LAYER_BOTTOM: Layer = Layer(20);
pub const WORLD_3D: Layer = Layer(100);
pub const ICED_WORLD: Layer = Layer(200);
pub const CAPTURE_DIM: Layer = Layer(300);
pub const CANVAS: Layer = Layer(400);
// 401/402 are RESERVED: scene.frame stacks detached (floating) viewport panes
// there (FLOATING_BG/FLOATING_CONTENT), directly above the tiled root's content.
/// Above ALL canvas content — tiled root windows AND floating panes. The band
/// artifact overlays (plugin quads, host-drawn artifact meshes) use to draw on
/// top of window content while staying under the compositor's own screen UI.
pub const CANVAS_ABOVE: Layer = Layer(410);
// Top/Overlay sit ABOVE windows but below the compositor's own screen iced
// (ICED_SCREEN) — again mirroring the hit-test priority (Iced Screen > Overlay > Top).
pub const LAYER_TOP: Layer = Layer(450);
pub const LAYER_OVERLAY: Layer = Layer(480);
pub const ICED_SCREEN: Layer = Layer(500);
pub const POINTER: Layer = Layer(700);

/// Transitional draw-node currency: type-erased until the concrete draw-node
/// model lands (phase 4 of document/ARCHITECTURE.md). The frame driver
/// downcasts to its renderer's node type when lowering.
pub type DrawNode = Box<dyn Any>;

/// Per-frame timing handed to `System::update`.
#[derive(Clone, Copy, Debug)]
pub struct FrameTick {
    /// Monotonic frame counter for this world.
    pub index: u64,
    /// Time since the previous tick of this world.
    pub delta: Duration,
}

/// What a frame is assembled from: each system pushes nodes at its declared
/// layers; the driver sorts (stable, so same-layer order = push order) and
/// lowers to the renderer.
#[derive(Default)]
pub struct FramePlan {
    items: Vec<(Layer, DrawNode)>,
}

impl FramePlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, layer: Layer, node: DrawNode) {
        self.items.push((layer, node));
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The plan in draw order (back to front).
    pub fn sorted(mut self) -> Vec<(Layer, DrawNode)> {
        self.items.sort_by_key(|(layer, _)| *layer);
        self.items
    }
}
