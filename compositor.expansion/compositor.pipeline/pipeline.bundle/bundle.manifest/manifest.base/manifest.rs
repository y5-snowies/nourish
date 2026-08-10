//! `pipeline.json` — the multipass shader-bundle manifest (pure serde model).
//!
//! A bundle is "multipass" when it carries a `pipeline.json` next to its format
//! folders. `parse` is tolerant at the call site: a bad manifest returns `Err`
//! and the loader falls back to the single-pass path (mirrors `shader.load`).

use serde::Deserialize;
use std::collections::BTreeMap;

/// The manifest filename a multipass bundle carries at its root.
pub const PIPELINE_FILE: &str = "pipeline.json";

/// A parsed `pipeline.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default = "one")]
    pub version: u32,
    /// Which heading this bundle sits under in the shader picker.
    ///
    /// A bundle may name its own; anything that does not — including every
    /// single-pass bundle, which has no manifest to say it in — lands under
    /// "User". The point is that a folder someone drops in is findable without
    /// having declared anything, while a curated set can group itself.
    #[serde(default)]
    pub category: Option<String>,
    /// Shared WGSL sources (relative to the bundle), importable via naga_oil `#import`.
    #[serde(default)]
    pub modules: Vec<String>,
    /// Named offscreen targets, besides the built-ins `content`/`windows`/`output`.
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
    /// Named GPU storage buffers the passes may read AND write, surviving from
    /// one frame to the next.
    ///
    /// The other half of `targets.persist`, and the half that buys SCATTER: a
    /// persistent target is written by rendering to it, so a pass can only write
    /// the pixel it is shading, while a storage buffer can be written at any
    /// index. That is what a histogram, a counter or a list needs and what a
    /// feedback image cannot express.
    ///
    /// It is also the more expensive half. Writing one from a fragment shader
    /// needs the device's `fragmentStoresAndAtomics` feature, so a bundle that
    /// declares any storage is refused outright on a device without it — the same
    /// treatment `window_textures` gets for descriptor indexing.
    #[serde(default)]
    pub storage: BTreeMap<String, Storage>,
    /// Still images the bundle ships beside its shaders, uploaded once and
    /// sampled by naming one in a pass's `inputs` — exactly as a target is named.
    ///
    /// This is the one input a shader cannot compute: photographic detail, hand-
    /// drawn art, a sprite atlas, a colour-grading LUT, a measured noise field.
    /// Everything else the engine offers is either generated or is the desktop
    /// itself.
    ///
    /// Cheap by construction, because it reuses the input path whole: a texture
    /// occupies a `@group(0)` binding like any other input, in the same sorted-by-
    /// binding-name order, so a pass sampling one is indistinguishable from a pass
    /// sampling a target. There is no new bind group, no push field and no engine
    /// work per frame — the cost is a decode and an upload, once, at load.
    #[serde(default)]
    pub textures: BTreeMap<String, Texture>,
    /// The passes, in execution order within each `when` band.
    pub passes: Vec<Pass>,
    /// Who composites client windows into the world band.
    #[serde(default)]
    pub windows: WindowMode,
    /// Suppress the engine's decoration border around each window. The border is
    /// drawn OUTSIDE the slot and is opaque, so any effect that reads window
    /// edges — a glow, a field between windows, a refraction — is reading the
    /// border rather than the window under it.
    #[serde(default)]
    pub decorations: Decorations,
    /// Suppress the black letterbox bars painted beside client content, so an
    /// undersized or translucent client composites against what is behind it.
    #[serde(default)]
    pub letterbox: LetterboxMode,
    /// The WGSL module holding this bundle's pointer warp, if it displaces the
    /// world band. See [`Hit`].
    #[serde(default)]
    pub hit: Option<Hit>,
}

/// Where the bundle's `hit_inverse` lives.
///
/// A shader that displaces the world band moves where things LOOK without moving
/// where they ARE, so the pointer lands on whatever occupies that spot in world
/// space rather than on what the user can see there. This names the function that
/// undoes it — and the SAME function the render pass samples through, so the two
/// cannot disagree.
///
/// It is a module path rather than an inline expression because the render pass
/// `#import`s it: one file, two readers. A bundle that displaces nothing omits
/// this and pays nothing.
///
/// The function must be `fn hit_inverse(uv: vec2<f32>, res: vec2<f32>,
/// params: vec4<f32>) -> vec2<f32>`, pure — no bindings, no textures, no globals.
/// `shader.hit` enforces that at load and says which rule was broken.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hit {
    /// Bundle-relative path to the WGSL module defining `hit_inverse`.
    pub module: String,
    /// Which `@prop`s land in the function's `params` argument, in order (up to
    /// four; the rest are zero).
    ///
    /// BY NAME, not by position. A pass's own `params` block is indexed by that
    /// PASS's declaration order, but the warp is bundle-level and the engine has
    /// only the union across passes — so "the first four props" means different
    /// things on the two sides the moment a bundle has more than one pass. Naming
    /// them removes the coupling: reorder passes or props and the warp still
    /// reads the value it asked for.
    #[serde(default)]
    pub params: Vec<String>,
    /// How the engine should serve the griddable entry. Ignored by
    /// `hit_drawable`, which is pointwise by necessity.
    #[serde(default)]
    pub evaluate: Evaluate,
}

/// How to serve `hit_inverse`. Not WHICH entry — that falls out of arity — but how
/// to answer a pure function of position.
///
/// The choice is an amortisation question, and the arithmetic decides it. A bake
/// computes `EDGE²` = 16k values; a pointer event consumes one. At 1 kHz polling
/// and 60 fps that is ~16 events per frame, so a bake only pays when its result
/// stays valid across more events than it has cells — roughly a thousand frames.
///
/// * `map_static` — bake once, look up. For a warp whose inputs (resolution, the
///   `@prop`s it names) hold still, which is the case that amortises. Bounded
///   per-event cost whatever the function costs; interpolation error bounded by
///   curvature × cell size, which `shader.map`'s tests hold under a thousandth of
///   the screen.
/// * `pointwise` — evaluate per event. Exact, no bake, no staleness. Cheap in
///   absolute terms for a CHEAP warp (~40 IR ops × ~16 events per frame for the
///   shipped barrel), and the only mode that can serve a DISCONTINUOUS one, since
///   both grid modes interpolate bilinearly between cells and a jump smeared
///   across a cell puts the pointer in the wrong region near every seam.
///
///   It is not the general answer for "animates", and reading it as one is how a
///   heavy warp becomes a stuttering cursor: the cost is the warp's own, in the
///   interpreter, on the input thread, at the polling rate. A warp with a loop or
///   a multi-octave field wants `map`, whose cost is one small pass per frame
///   whatever the function does. Authoring guidance lives in
///   `document/shader-skill/SKILL.md` §8; keep the two in step.
/// * `map` — produced on the GPU, every frame. For a warp that BOTH animates (so a
///   static bake is invalid immediately) and is expensive enough per evaluation
///   that pointwise stops being free. The GPU renders the grid as part of the frame
///   and the result is read back; the pointer pays a lookup and the map trails by
///   one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evaluate {
    #[default]
    MapStatic,
    Pointwise,
    Map,
}

/// Whether the engine draws its own window border. `keep` (default) is what
/// every bundle got before this switch existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decorations { #[default] Keep, Off }

/// When to suppress the letterbox fill: never (`keep`), only while a resize is in
/// flight (`on-resize`), or entirely (`always`).
///
/// **Currently inert** — parsed and carried, but `window.draw.frame::scene` takes
/// the same arm for every mode. Both suppressing modes existed to escape the
/// full-slot resize backstop, which no longer exists; the arms are commented out
/// there rather than deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LetterboxMode { #[default] Keep, OnResize, Always }

/// Who draws the world band. `engine` (default) blits it into `content` as
/// today. The other two leave part or all of it OUT of `content` and publish the
/// world set instead, so a pass composites it itself — the prerequisite for
/// transparency, rotation or any per-drawable geometry, since a flattened
/// `content` has already destroyed whatever was behind a window.
/// See `SHADER_PIPELINE.md` §8d.
///
/// # Which to pick
///
/// `world` unless you have a reason not to. `pipeline` hands the bundle client
/// windows ONLY, and the engine keeps drawing the rest of the band — iced-world
/// placeholders, group frames, whatever the world grows next — after the
/// pipeline's output pass. That is not a z-fight, it is unconditional: those
/// drawables land on top of every window the bundle drew, however deep they
/// actually are. `world` hands over the whole band in `drawable_order()`, so the
/// bundle can reproduce the engine's order exactly (and then break it on
/// purpose, which is usually the point).
///
/// `pipeline` stays because bundles were written against it and its set is
/// exactly the client windows — no `kind` test needed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMode { #[default] Engine, Pipeline, World }

/// An offscreen intermediate image. `scale` is the edge length as a fraction of
/// the output (0.5 = half-res).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(default)]
    pub format: Format,
    #[serde(default = "one_f32")]
    pub scale: f32,
    /// Keep this target's contents from one frame to the next: a pass that
    /// samples it reads what was written LAST frame, not this one.
    ///
    /// Implemented as a pair of images the engine alternates, not as a
    /// preserved attachment — reading and writing one image in a single pass is
    /// a feedback loop Vulkan does not allow without an extension, and the
    /// alternation costs nothing beyond the second image because a fullscreen
    /// pass writes every pixel anyway.
    ///
    /// The contents are VOLATILE: both images are zeroed when they are allocated,
    /// and they are reallocated whenever the output size, the swapchain format or
    /// the selected bundle changes. This is an accumulator, not a save file.
    #[serde(default)]
    pub persist: bool,
}

/// A GPU storage buffer: read-write, zeroed once, and kept across frames.
///
/// Bound to EVERY pass of the bundle at `@group(2)`, in the sorted order of the
/// names — all of them, not a per-pass subset. Per-pass selection would be a
/// second place for the binding order to be stated and therefore a second place
/// for it to be wrong, and an unused binding costs a descriptor, not a dispatch.
///
/// Volatile in exactly the way a persistent target is: zeroed at allocation, and
/// reallocated whenever the output size, the swapchain format or the selected
/// bundle changes.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    /// Size in bytes. Rounded up to 16 so a `vec4`-strided array never runs off
    /// the end of its own last element.
    pub bytes: u64,
}

/// A still image, decoded and uploaded once when the bundle is selected.
///
/// # `srgb`
///
/// The one attribute that cannot be defaulted correctly for every file, because
/// it says what the bytes MEAN rather than how they are stored. Artwork is
/// sRGB-encoded and must be linearised on sample or it composites too bright
/// against a linear graph. A file read as DATA — a LUT, a mask, a height or
/// normal field, a noise table — carries numbers that happen to be in an image,
/// and linearising them corrupts every one of them.
///
/// So: `true` (the default) for anything drawn to be looked at, `false` for
/// anything sampled to be used. Getting it wrong is subtly-wrong output, not an
/// error, which is exactly why it is declared and not guessed.
///
/// # Alpha is STRAIGHT, not premultiplied
///
/// A sampled texel is `(r, g, b, a)` as authored. Nothing here premultiplies,
/// deliberately: a shader is the only thing that ever sees these texels — the
/// fixed-function blend applies to a pass's OUTPUT, never to a texture read —
/// and premultiplying at upload would have to happen in the file's encoded
/// space, where `srgb_decode(rgb * a) != srgb_decode(rgb) * a` darkens every
/// soft edge. A pass compositing a sprite multiplies by alpha itself, in linear
/// space, where it is correct.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Texture {
    /// Bundle-relative path to a PNG or JPEG. Must stay inside the bundle: unlike
    /// `modules`, which share sources between sibling bundles on purpose, a
    /// texture path may not contain `..` or be absolute.
    pub file: String,
    /// Whether the file's colours are sRGB-encoded. See the type doc.
    #[serde(default = "yes")]
    pub srgb: bool,
}

/// One fullscreen pass. `inputs` maps a binding name → the target it samples
/// (built-in targets: `content`, `windows`, `history` — the last two are
/// post-composite, so after-content only); `output` names the target written
/// (built-in `output` = the swapchain/dmabuf).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pass {
    pub name: String,
    /// WGSL fragment source, relative to the bundle dir.
    pub shader: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    #[serde(default = "output_name")]
    pub output: String,
    #[serde(default)]
    pub when: When,
    /// naga_oil shader-defs set while compiling this pass (variant selection).
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Engine resources this pass requires — and, one for one, the engine costs
    /// it authorises. See [`Requirement`]; nothing is inferred from use.
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// Where this pass should execute (`auto` default).
    #[serde(default)]
    pub place: Place,
    /// Run this pass only every Nth frame, holding its target in between (1 =
    /// every frame). Only meaningful on a pass writing an intermediate target —
    /// the `output` pass must run every frame or the frame has no picture, which
    /// `plan()` enforces. See `SHADER_PIPELINE_WORKER.md` stage 6.
    #[serde(default = "one")]
    pub cadence: u32,
}

/// Where a pass executes. `auto` puts it on the off-thread background worker
/// when it crosses no device boundary, else inline. `worker` asks for the
/// worker explicitly and is downgraded (loudly) when that is impossible.
/// Resolved by `shader.place`; see `document/SHADER_PIPELINE_WORKER.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place { #[default] Auto, Worker, Compositor }

/// Intermediate-target pixel format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format { Rgba8, #[default] Rgba16f, Rgba32f }

/// Where a pass runs relative to window compositing. `after-content` samples the
/// composited `content` target (vignette-above, tone-map).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum When { #[default] BeforeContent, AfterContent }

/// What a pass requires the engine to provide, and the cost each entry turns on.
///
/// # `window_*` vs `world_*`
///
/// Same two bindings; different MEMBERSHIP. `window_geometry` / `window_textures`
/// carry client windows only — what an effect that is *about windows* wants
/// (`window-glow` haloing a placeholder would be wrong).
///
/// `world_geometry` / `world_textures` carry every world drawable, panels
/// included, each tagged with its kind. An effect that REPLACES pixels rather than
/// decorating them needs this, and `glass` is the case that shows why: it finds
/// the front-most rect covering a pixel and re-composites that surface over
/// `content`. Handed windows only, a placeholder stacked ON a window is not in the
/// set, so glass paints the window back over pixels the engine had already
/// composited correctly — and the placeholder appears to fall behind the window.
/// Widening the set fixes it with no shader change: the same "last containing rect
/// wins" loop now sees the panel and picks it.
///
/// The `world_*` pair is only meaningful under `windows: engine`. An ownership
/// mode already decides membership, and asking for the world set while owning only
/// the windows would mean drawing panels the engine also draws — `plan()` rejects
/// that.
pub use compositor_pipeline_bundle_require_base::require::{Requirement, Requires};

/// The union of what every pass in `m` requires. THE gate for every engine cost —
/// see `Requires`.
pub fn requires(m: &Manifest) -> Requires {
    Requires::of(m.passes.iter().flat_map(|p| p.requires.iter().copied()))
}

/// Parse a `pipeline.json` string into a `Manifest`.
pub fn parse(src: &str) -> Result<Manifest, String> {
    serde_json::from_str(src).map_err(|e| format!("pipeline.json: {e}"))
}

fn one() -> u32 { 1 }
fn yes() -> bool { true }
fn one_f32() -> f32 { 1.0 }
fn output_name() -> String { "output".to_string() }
