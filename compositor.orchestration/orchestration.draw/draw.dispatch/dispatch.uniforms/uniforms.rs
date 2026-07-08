use smithay::backend::renderer::gles::{GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexture, Uniform};
use smithay::backend::renderer::{Frame, Renderer, RendererSuper};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size};
use std::borrow::Cow;

/// Renderer-agnostic uniform values for the parallax background shader. The GLES
/// path ignores these (it uses named `Uniform`s + its compiled `GlesPixelProgram`);
/// a renderer with a native background shader (Vulkan) drives its pipeline from them.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParallaxUniforms {
    pub resolution: [f32; 2],
    pub zoom: f32,
    pub time: f32,
    pub pan: [f32; 2],
    pub flow_offset: [f32; 2],
    /// Smoothed pan velocity (world px/s). Fed to native shaders as two f16
    /// halves packed into the `lock_alpha.w` push lane (`lock_alpha.z` carries
    /// the sRGB flag) so velocity-reactive backgrounds (metaballs) can stretch
    /// along motion. Shaders that ignore it are unaffected.
    pub velocity: [f32; 2],
    pub lock_amount: f32,
    pub alpha: f32,
    pub srgb: f32, // push lock_alpha.z: 0 = raw output, 1 = gamma-encode to sRGB
}

/// One compiled fullscreen-shader variant a renderer can run: a SPIR-V module
/// plus the push-constant payload for this frame. Renderer-agnostic and
/// shader-agnostic — a renderer that owns a native fullscreen pipeline (Vulkan)
/// builds/caches a pipeline keyed by `id` and draws it with `push`; renderers
/// without that path ignore it. The producing scene element owns the shader
/// bytes and the push layout, so no shader-specific knowledge leaks into the
/// renderer. `Cow` so both built-in (`'static`) and runtime-compiled (owned)
/// shader bytes flow through the same seam.
#[derive(Clone)]
pub struct ShaderVariant<'a> {
    /// Stable per-shader id, used as the renderer's pipeline-cache key.
    pub id: u64,
    /// SPIR-V module bytes. Holds both entry points unless `vert_spv` is set.
    pub spv: Cow<'a, [u8]>,
    /// Separate vertex-stage SPIR-V module (set when the fragment was compiled
    /// alone, e.g. a `glsl/` bundle paired with a fullscreen vertex).
    pub vert_spv: Option<Cow<'a, [u8]>>,
    pub vert_entry: Cow<'a, str>,
    pub frag_entry: Cow<'a, str>,
    /// Push-constant bytes for this draw (already packed by the producer).
    pub push: Cow<'a, [u8]>,
}

/// A renderer-native fullscreen-shader draw handed through the dispatch seam:
/// the standard (SDR) variant plus an optional variant the renderer selects
/// when compositing for HDR output.
#[derive(Clone)]
pub struct NativeShaderPass<'a> {
    pub sdr: ShaderVariant<'a>,
    pub hdr: Option<ShaderVariant<'a>>,
}

/// Per-renderer draw seam for scene elements that carry GLES-produced resources
/// (iced UI, bevy 3D, parallax pixel shader).
///
/// The trait is on the **renderer** `R` (a plain bound — `R: SceneDispatch`),
/// not on `R::Frame`, to avoid the GAT higher-ranked-lifetime limitation
/// (rust#100013) that a `for<'a,'b> R::Frame<'a,'b>: Trait` bound runs into at
/// the use site. The methods take the frame as a parameter.
///
/// - `GlesRenderer` implements it for real (renders the texture / runs the pixel

/// GLES body for `SceneDispatch::draw_prerendered_texture` (delegated from the
/// trait impl in dispatch.frame, which the orphan rule pins to the trait crate).
pub fn draw_prerendered_texture(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    src: Rectangle<f64, BufferCoord>,
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    alpha: f32,
) -> Result<(), <GlesRenderer as RendererSuper>::Error> {
    Frame::render_texture_from_to(
        frame, texture, src, dst, damage, &[], smithay::utils::Transform::Normal, alpha,
    )
}

/// GLES body for `SceneDispatch::draw_pixel_program`.
#[allow(clippy::too_many_arguments)]
pub fn draw_pixel_program(
    frame: &mut GlesFrame<'_, '_>,
    program: Option<&GlesPixelProgram>,
    src: Rectangle<f64, BufferCoord>,
    dst: Rectangle<i32, Physical>,
    size: Size<i32, BufferCoord>,
    damage: &[Rectangle<i32, Physical>],
    alpha: f32,
    uniforms: &[Uniform<'_>],
) -> Result<(), <GlesRenderer as RendererSuper>::Error> {
    // The program is `None` when the element was built while the compositor
    // preferred dmabuf/Vulkan (the GLES pixel program is skipped then). If the
    // GLES path is nonetheless reached — e.g. a runtime Vulkan→GLES fallback flips
    // the render path after the element was prepared — skip the parallax this frame
    // instead of crashing; the next prepare() rebuilds it with a compiled program.
    let Some(program) = program else {
        return Ok(());
    };
    frame.render_pixel_shader_to(program, src, dst, size, Some(damage), alpha, uniforms)
}
