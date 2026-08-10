use smithay::backend::renderer::gles::{GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexture, Uniform};
use smithay::backend::renderer::{Frame, Renderer, RendererSuper};
use smithay::utils::{Buffer as BufferCoord, Physical, Rectangle, Size};
use std::sync::Arc;

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
/// renderer.
///
/// The module bytes are `Arc`, not `Cow`: this seam is crossed on every draw
/// call of every frame, but the renderer only reads the bytes once, when its
/// pipeline cache misses. `Arc` lets both built-in (`'static`) and
/// runtime-compiled shaders flow through the same seam while making the
/// per-frame hand-off a refcount bump rather than a deep copy of the blob.
/// `push` is an `Arc` too — it is ~112 bytes and differs per draw, but every
/// consumer took ownership of it anyway, and the lifetime a `Cow` needed is what
/// kept this seam off `'static`.
#[derive(Clone)]
pub struct ShaderVariant {
    /// Stable per-shader id, used as the renderer's pipeline-cache key.
    pub id: u64,
    /// SPIR-V module bytes. Holds both entry points unless `vert_spv` is set.
    pub spv: Arc<[u8]>,
    /// Separate vertex-stage SPIR-V module (set when the fragment was compiled
    /// alone, e.g. a `glsl/` bundle paired with a fullscreen vertex).
    pub vert_spv: Option<Arc<[u8]>>,
    pub vert_entry: Arc<str>,
    pub frag_entry: Arc<str>,
    /// Push-constant bytes for this draw (already packed by the producer).
    ///
    /// `Arc`, not `Cow`. The borrow saved nothing — every consumer called
    /// `into_owned()` on it immediately — while the lifetime it introduced spread
    /// through `PipelinePass` and `ShaderPipeline` and kept the whole seam off
    /// `'static`, which is what an opaque handle needs.
    pub push: Arc<[u8]>,
}

/// A renderer-native fullscreen-shader draw handed through the dispatch seam:
/// the standard (SDR) variant plus an optional variant the renderer selects
/// when compositing for HDR output. `pipeline`, when set, is a multipass graph
/// the renderer runs INSTEAD of the single `sdr` pass (Vulkan only; renderers
/// without a graph executor ignore it and use `sdr`).
#[derive(Clone)]
pub struct NativeShaderPass {
    pub sdr: ShaderVariant,
    pub hdr: Option<ShaderVariant>,
    /// A multipass bundle for the renderer to run INSTEAD of `sdr`, as an OPAQUE
    /// handle.
    ///
    /// This crate describes how the scene hands a renderer a shader pass; it does
    /// not know what a multipass pipeline is, and it used to declare seven types
    /// that existed for nothing else. They live in `pipeline.abi/abi.seam` now,
    /// and travel through here without being named — which is what lets the
    /// pipeline grow what it carries without touching orchestration at all.
    ///
    /// `'static` by construction: the seam types hold `Arc`s, so a renderer that
    /// understands the handle downcasts it and one that does not ignores it.
    pub pipeline: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
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
