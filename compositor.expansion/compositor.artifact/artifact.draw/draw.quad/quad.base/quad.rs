//! `ArtifactQuad` — the world/screen-placed quad an artifact `System` contributes.
//! Lives in the artifact expansion (NOT orchestration): the frame driver consumes it
//! the same way it consumes the expansion-owned `ParallaxBackground`.
//!
//! Placement is declared in the quad's own SPACE and projected by the frame driver
//! at plan time (`scene.frame`): the contributor never pre-projects. The rect is
//! kept as raw `f64`s (not a smithay-marked type) because its unit depends on
//! `space` — marking it `Logical`/`Physical` would lie for the other space:
//!
//! - `ArtifactSpace::World` — world-logical units: pans/zooms with the camera;
//!   sizes scale with zoom.
//! - `ArtifactSpace::Screen` — physical screen pixels: fixed on screen.
//!
//! `w`/`h` are the on-screen viewport of the content: a `Dmabuf`'s texture is
//! stretched to fill it (any stretch — no implicit natural-size/zoom coupling).

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Rectangle};

/// The pixels of an artifact quad. `Solid` is renderer-agnostic and needs NO GPU
/// allocation; `Dmabuf` carries content the artifact rendered itself, imported
/// zero-copy at the single `lower()` seam and stretched to the quad's rect.
#[derive(Clone)]
pub enum ArtifactContent {
    Solid([f32; 4]),
    Dmabuf(Dmabuf),
}

/// Which space the quad's rect is declared in (see module docs for units).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArtifactSpace {
    /// World-logical; the frame driver projects through the active camera every
    /// frame (pan/zoom follow for free).
    World,
    /// Physical screen pixels; composited as-is.
    Screen,
}

/// One quad. `id` must be stable across frames for damage tracking; bump `commit`
/// when the content changes (animated contributors typically use fresh ids instead).
#[derive(Clone)]
pub struct ArtifactQuad {
    pub content: ArtifactContent,
    pub space: ArtifactSpace,
    /// Position (top-left) in the space's units.
    pub x: f64,
    pub y: f64,
    /// On-screen viewport size of the content, in the space's units.
    pub w: f64,
    pub h: f64,
    pub id: Id,
    pub commit: CommitCounter,
}

impl ArtifactQuad {
    /// A solid quad in WORLD space (world-logical rect).
    pub fn solid_world(color: [f32; 4], x: f64, y: f64, w: f64, h: f64, id: Id, commit: CommitCounter) -> Self {
        Self { content: ArtifactContent::Solid(color), space: ArtifactSpace::World, x, y, w, h, id, commit }
    }

    /// A solid quad fixed on SCREEN (physical px).
    pub fn solid_screen(color: [f32; 4], x: f64, y: f64, w: f64, h: f64, id: Id, commit: CommitCounter) -> Self {
        Self { content: ArtifactContent::Solid(color), space: ArtifactSpace::Screen, x, y, w, h, id, commit }
    }

    /// A dmabuf quad; the texture is stretched to fill the rect (any stretch).
    #[allow(clippy::too_many_arguments)]
    pub fn dmabuf(dmabuf: Dmabuf, space: ArtifactSpace, x: f64, y: f64, w: f64, h: f64, id: Id, commit: CommitCounter) -> Self {
        Self { content: ArtifactContent::Dmabuf(dmabuf), space, x, y, w, h, id, commit }
    }
}

/// A quad AFTER plan-time projection: a physical rect + a `screen` flag feeding the
/// per-element meta (so screen artifacts aren't treated as world content by e.g. the
/// Vulkan AA pass). Built by the frame driver, lowered by `draw.node`.
#[derive(Clone)]
pub struct ProjectedArtifact {
    pub content: ArtifactContent,
    pub rect: Rectangle<i32, Physical>,
    pub screen: bool,
    pub id: Id,
    pub commit: CommitCounter,
}
