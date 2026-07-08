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
/// Grow this with new per-element facts the renderer needs (e.g. an explicit
/// AA-eligibility flag) — today it's just the element's space, from which the
/// renderer derives things like "apply AA only to world content".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ElementMeta {
    pub space: ElementSpace,
}

impl ElementMeta {
    pub const WORLD: ElementMeta = ElementMeta { space: ElementSpace::World };
    pub const SCREEN: ElementMeta = ElementMeta { space: ElementSpace::Screen };

    /// World content (windows + iced-world) — currently the AA-eligible set.
    pub fn is_world(self) -> bool {
        matches!(self.space, ElementSpace::World)
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
        pass: NativeShaderPass<'_>,
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
        _pass: NativeShaderPass<'_>,
    ) -> Result<(), <Self as RendererSuper>::Error> {
        gles::draw_pixel_program(frame, program, src, dst, size, damage, alpha, uniforms)
    }
}
