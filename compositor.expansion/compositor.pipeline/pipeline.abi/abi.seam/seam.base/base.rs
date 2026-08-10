//! The multipass seam: what a compiled bundle hands a renderer for one draw.
//!
//! # Why these are here and not in `dispatch.uniforms`
//!
//! They were all defined in `orchestration.draw/draw.dispatch/dispatch.uniforms`,
//! whose job is the renderer-agnostic draw seam for scene elements — iced, bevy,
//! and the parallax pixel shader. Seven of its twelve public items existed only
//! for this feature, and its first line re-exported `Requires` from a pipeline
//! crate: an orchestration crate importing the vocabulary of an expansion that
//! sits above it, so that it could describe that expansion to the renderer.
//!
//! Nothing upstream needs to know what a pass or an intermediate target is. The
//! seam carries them as an opaque handle (`NativeShaderPass::pipeline`), the
//! renderer that understands one downcasts it, and these types belong to the
//! feature that defines them.

pub use compositor_pipeline_bundle_require_base::require::{Requirement, Requires};
use compositor_orchestration_draw_dispatch_frame::ShaderVariant;

/// Intermediate-target pixel format for a multipass pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetFormat {
    Rgba8,
    Rgba16f,
    Rgba32f,
}

/// An intermediate offscreen target: format + edge length as a fraction of the output.
#[derive(Clone, Copy, Debug)]
pub struct PipelineTarget {
    pub format: TargetFormat,
    pub scale: f32,
    /// Double-buffered by the executor: a pass sampling it reads the previous
    /// frame's contents. See `shader.manifest::Target::persist`.
    pub persist: bool,
}

/// A bundle texture: decoded RGBA8 plus what the samples mean.
///
/// Pixels rather than a path, because the renderer that uploads these may not be
/// the process's own — the off-thread background worker runs the same executor on
/// its own device — and a path would make each of them decode the file again,
/// with no guarantee the two got the same bytes. `Arc` for the same reason a
/// pass's SPIR-V is one: this crosses the seam every frame and is read on the
/// first.
#[derive(Clone)]
pub struct PipelineTexture {
    pub width: u32,
    pub height: u32,
    /// sRGB-encoded samples, so the renderer picks a `_SRGB` image format and the
    /// hardware linearises on read. See `shader.manifest::Texture::srgb`.
    pub srgb: bool,
    /// Tightly packed RGBA8, top row first, STRAIGHT alpha.
    pub pixels: std::sync::Arc<[u8]>,
}

/// Where a multipass pass writes: an intermediate target by index, or the swapchain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineOutput {
    Target(usize),
    Swapchain,
}

/// One pass of a multipass pipeline: its compiled variant (SPIR-V + packed push)
/// plus resolved input indices (in binding order) and output. An input index of
/// `usize::MAX` samples the composited `content` (after-content passes only).
#[derive(Clone)]
pub struct PipelinePass {
    pub variant: ShaderVariant,
    pub inputs: Vec<usize>,
    pub output: PipelineOutput,
    /// What THIS pass requires bound: `world_set()` binds the geometry UBO at
    /// `@group(1) @binding(0)`, `textures()` also binds the bindless array at
    /// `@binding(1)`.
    pub requires: Requires,
    /// Run every Nth frame, holding the target in between (1 = every frame).
    /// Intermediates only — the `output` pass always runs.
    pub cadence: u32,
}

/// A multipass background pipeline (Vulkan-only): named intermediate targets plus
/// the passes per band. `passes` are `before-content`; `after` run once the scene
/// is composited into `content`, which they may sample.
#[derive(Clone)]
pub struct ShaderPipeline {
    /// Stable id (pipeline-cache key base for the renderer's per-pass pipelines).
    pub id: u64,
    pub targets: Vec<PipelineTarget>,
    /// Declared storage-buffer sizes in bytes, in binding order at `@group(2)`.
    /// Empty for a bundle that declares none, which is what keeps the descriptor
    /// set from being built.
    pub storage: Vec<u64>,
    /// Declared textures, in sorted-name order — which IS the order a pass's
    /// input index refers to them by (see the `TEXTURE` sentinel in
    /// `renderer.graph`). Empty for every bundle that declares none, which is what
    /// keeps the upload path from existing.
    pub textures: Vec<PipelineTexture>,
    pub passes: Vec<PipelinePass>,
    pub after: Vec<PipelinePass>,
    /// The union of every pass's `requires` — THE authority on what this pipeline
    /// costs the engine, and the only reason the engine does anything extra for
    /// it. `previous_frame`/`window_layer`/`composited_scene` each gate one
    /// offscreen image; `world_set()` gates collecting the world set at all;
    /// `whole_band()` widens its membership; `textures()` gates the bindless array.
    ///
    /// Carried as the bundle's own vocabulary rather than as engine booleans so a
    /// gate here and the manifest entry that authorised it are the same word. The
    /// previous shape — five independent bools — is how a cost ends up with no
    /// entry behind it.
    pub requires: Requires,
    /// How much of the world band the renderer must NOT draw, because this
    /// pipeline composites it itself from the world-texture array. See
    /// `SHADER_PIPELINE.md` §8d.
    pub owns: WorldOwn,
    /// The generated grid pass for a per-frame GPU warp map (`evaluate: "map"`),
    /// when the bundle asked for one. `None` for every other bundle, which is what
    /// keeps the target, the staging buffer and the readback from existing at all.
    pub warp_map: Option<ShaderVariant>,
}

/// How much of the world band a bundle takes over.
///
/// The renderer's suppression and the set the shader is handed are BOTH derived
/// from this one value, and that identity is the point: whatever the bundle is
/// not given, the engine still draws — after the pipeline's output pass, hence on
/// top of everything the bundle drew. Any gap between the two shows up as world
/// content floating over windows it belongs under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorldOwn {
    /// The engine draws the whole band (`windows: engine`).
    #[default]
    Engine,
    /// `windows: pipeline` — client windows only. Iced-world panels
    /// (placeholders, groups) are still engine-drawn and therefore composite ON
    /// TOP of the bundle's windows regardless of their real depth. Kept for the
    /// bundles written against it; [`WorldOwn::World`] is the ordering-correct
    /// form.
    Windows,
    /// `windows: world` — the entire world band, in `drawable_order()`, drawn by
    /// the bundle. The engine draws none of it, so composite order is whatever
    /// the shader makes it, and iterating the set front-to-back reproduces the
    /// engine's own order exactly.
    World,
}

/// The renderer's spelling of the same three-way choice.
///
/// One conversion, in one place. There were four hand-written 3-arm matches, two of
/// them copies of each other a hundred lines apart, and the arms do not even share
/// names (`WorldOwn::Engine` is `Own::None`, `WorldOwn::Windows` is
/// `Own::Windows`). Adding a fourth mode meant finding all four and hoping; now it
/// is one non-exhaustive match that will not compile.
impl From<WorldOwn> for compositor_pipeline_abi_worldset_base::base::Own {
    fn from(o: WorldOwn) -> Self {
        use compositor_pipeline_abi_worldset_base::base::Own;
        match o {
            WorldOwn::Engine => Own::None,
            WorldOwn::Windows => Own::Windows,
            WorldOwn::World => Own::World,
        }
    }
}

/// Recover a [`ShaderPipeline`] from the seam's opaque handle.
///
/// `NativeShaderPass::pipeline` is an `Arc<dyn Any>` so that `dispatch.uniforms`
/// can carry a multipass bundle to a renderer without naming one. Both consumers
/// — the compositor's Vulkan renderer and the off-thread worker — recover it
/// here rather than each writing its own downcast, so there is one place that
/// knows what the handle is and one answer when it is something else.
///
/// `None` means either "no bundle" or "a handle this build does not recognise";
/// the caller distinguishes them if it cares, and both mean "run the single pass".
pub fn as_pipeline(
    handle: Option<&std::sync::Arc<dyn std::any::Any + Send + Sync>>,
) -> Option<&ShaderPipeline> {
    handle?.downcast_ref::<ShaderPipeline>()
}
