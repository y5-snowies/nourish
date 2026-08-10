use smithay::backend::renderer::gles::{GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexture, Uniform};
use smithay::backend::renderer::{Renderer, RendererSuper};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size};

pub use compositor_orchestration_draw_dispatch_uniforms::uniforms::{
    NativeShaderPass, ParallaxUniforms, ShaderVariant,
};

/// Which coordinate space an element lives in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ElementSpace {
    /// Screen-space / output-fixed: iced screen UI, backgrounds, pointer,
    /// layershell. The default for anything not explicitly tagged.
    #[default]
    Screen,
    /// Pannable-world content: client windows + iced-world panels.
    World,
}

/// Per-element render metadata carried from scene assembly to the renderer.
/// Grow this with new per-element facts the renderer needs — today: the
/// element's `space` (drives "apply AA only to world content") and whether the
/// element is a client `window` (drives the shader pipeline's window-set — only
/// client windows feed window-rects/window-textures, never iced-world panels or
/// placeholders). `window` implies `space == World`.
/// No `Eq`: `times` is floating point. Nothing compares an `ElementMeta` — it is
/// carried, not matched — so the bound was free to drop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ElementMeta {
    pub space: ElementSpace,
    /// True only for client windows — the subset of world content the shader
    /// pipeline samples as "windows". Iced-world panels/placeholders are
    /// world-space but NOT windows.
    pub window: bool,
    /// True only for the background band element. Says the frame HAS a band, not
    /// who owns it — ownership is a property of the loaded bundle
    /// (`set::owns_band`). A bundle whose worker never produced a frame lowers no
    /// element, and the engine must then keep drawing the windows itself rather
    /// than suppress them for a band that is not there.
    pub background: bool,
    /// What the compositor knows about this element's window — focus, stacking,
    /// fullscreen, resize — as `window.descriptor` bits, reaching the shader in
    /// the `attrs` lane.
    ///
    /// Raw bits so this crate keeps naming nothing in the kernel; the vocabulary
    /// lives with the packer. Zero for everything that is not a client window,
    /// which is why it does not need to be an `Option`: no window, nothing known.
    ///
    /// This is the reason the meta for a canvas node is BUILT rather than a
    /// constant. Every other field is a property of the node's kind and could be
    /// decided at `lower()` time from the variant alone; these are properties of
    /// the window behind it, and by `lower()` the window is gone. They are filled
    /// where the window is still in hand and carried the rest of the way.
    pub flags: u32,
    /// When each thing last happened to this element's window, on the shared
    /// clock (`window.clock`), in `window.descriptor::Times` order.
    ///
    /// All `NEVER` unless the active bundle declared `window_times` AND this is a
    /// client window. Carried by value like the rest of the meta: it is 32 bytes
    /// against a draw call, and the alternative — a side table joined later — is
    /// the same index-drift the world set was built to avoid.
    pub times: [f32; 12],
}

/// Nothing has happened. `-1.0` is `window.clock::NEVER`, restated as a literal
/// because these are `const` and that crate is below this one, not beside it.
const NONE: [f32; 12] = [-1.0; 12];

impl ElementMeta {
    /// Iced-world content (panels, placeholders): world-space, not a window.
    pub const WORLD: ElementMeta =
        ElementMeta { space: ElementSpace::World, window: false, background: false, flags: 0, times: NONE };
    /// A client window: world-space AND part of the shader window-set. Carries no
    /// descriptors — [`ElementMeta::window`] is the form that does, and this stays
    /// for the paths that draw a window without one in hand (the overview grid).
    pub const WINDOW: ElementMeta =
        ElementMeta { space: ElementSpace::World, window: true, background: false, flags: 0, times: NONE };
    pub const SCREEN: ElementMeta =
        ElementMeta { space: ElementSpace::Screen, window: false, background: false, flags: 0, times: NONE };

    /// The background band. Screen-spaced like any other background, so nothing
    /// about AA or the band split changes.
    pub const BACKGROUND: ElementMeta =
        ElementMeta { space: ElementSpace::Screen, window: false, background: true, flags: 0, times: NONE };

    /// A client window with its descriptors attached.
    pub const fn window(flags: u32, times: [f32; 12]) -> ElementMeta {
        ElementMeta { space: ElementSpace::World, window: true, background: false, flags, times }
    }

    pub fn is_background(self) -> bool {
        self.background
    }

    /// World content (client windows + iced-world) — the AA-eligible set.
    pub fn is_world(self) -> bool {
        matches!(self.space, ElementSpace::World)
    }

    /// A client window — the subset of world content the shader pipeline feeds
    /// into window-rects/window-textures. Excludes iced-world panels and
    /// placeholders (they stay `is_world()` for AA but must not be treated as
    /// windows by glass/window-glow).
    pub fn is_window(self) -> bool {
        self.window
    }
}
use compositor_orchestration_draw_dispatch_uniforms::uniforms as gles;

/// The per-renderer dispatch seam keeping GLES-welded scene elements (iced UI,
/// bevy 3D, the parallax background) renderer-agnostic. GLES implements it for
/// real; other renderers (Vulkan) implement a blank draw until their native
/// path lands. Taken as a param-on-renderer trait to dodge the GAT-HRTB
/// limitation (rust#100013).
pub trait SceneDispatch: Renderer {
    /// Whether scene composition should feed this renderer the iced/bevy/parallax
    /// output as a dmabuf (native texture) instead of the GLES-welded seam.
    fn prefers_dmabuf() -> bool {
        false
    }

    /// Hand the renderer the worker's DECORATED world band for this frame.
    ///
    /// Stage 4 is a round trip: the compositor's composited `content` goes out to
    /// the worker, and a full band (background, windows, effect) comes back to be
    /// presented INSTEAD of running the after pass here. This is the return leg.
    ///
    /// It went through a process-global slot that the presenting frame TOOK, on
    /// the reasoning that a stale band would freeze the desktop. Taking is the
    /// right lifetime and it is kept — but the slot was shared by every pane, so
    /// the first `submit_frame` of a multi-output frame stole the band produced
    /// for another one. Handed along the seam, it reaches the renderer that is
    /// about to present it and no other.
    ///
    /// Default no-op: only the Vulkan path presents a worker band.
    fn set_after_band(&mut self, _band: Option<smithay::backend::allocator::dmabuf::Dmabuf>) {}

    /// Hand the renderer the metadata for the element about to be drawn (its
    /// space, and whatever else `ElementMeta` grows). The scene wrapper calls
    /// this before each element's `draw`, letting a renderer restrict effects
    /// like anti-aliasing to world content. Default no-op (renderers that don't
    /// need it ignore it).
    fn set_element_meta(_frame: &mut <Self as RendererSuper>::Frame<'_, '_>, _meta: ElementMeta) {}

    /// Draw a pre-rendered GLES texture into `frame`. Blank for renderers that
    /// cannot sample a `GlesTexture`.
    fn draw_prerendered_texture(
        frame: &mut <Self as RendererSuper>::Frame<'_, '_>,
        texture: &GlesTexture,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        alpha: f32,
    ) -> Result<(), <Self as RendererSuper>::Error>;

    /// Run a GLES pixel-shader program over a region. Blank for renderers
    /// without a pixel-shader path; `program` is `None` for those. `pass`
    /// carries a renderer-native fullscreen-shader draw (SPIR-V + push bytes)
    /// for renderers that composite the background with their own pipeline.
    #[allow(clippy::too_many_arguments)]
    fn draw_pixel_program(
        frame: &mut <Self as RendererSuper>::Frame<'_, '_>,
        program: Option<&GlesPixelProgram>,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        size: Size<i32, BufferCoord>,
        damage: &[Rectangle<i32, Physical>],
        alpha: f32,
        uniforms: &[Uniform<'_>],
        pass: NativeShaderPass,
    ) -> Result<(), <Self as RendererSuper>::Error>;
}

impl SceneDispatch for GlesRenderer {
    fn draw_prerendered_texture(
        frame: &mut GlesFrame<'_, '_>,
        texture: &GlesTexture,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        alpha: f32,
    ) -> Result<(), <Self as RendererSuper>::Error> {
        gles::draw_prerendered_texture(frame, texture, src, dst, damage, alpha)
    }

    fn draw_pixel_program(
        frame: &mut GlesFrame<'_, '_>,
        program: Option<&GlesPixelProgram>,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        size: Size<i32, BufferCoord>,
        damage: &[Rectangle<i32, Physical>],
        alpha: f32,
        uniforms: &[Uniform<'_>],
        _pass: NativeShaderPass,
    ) -> Result<(), <Self as RendererSuper>::Error> {
        gles::draw_pixel_program(frame, program, src, dst, size, damage, alpha, uniforms)
    }
}
