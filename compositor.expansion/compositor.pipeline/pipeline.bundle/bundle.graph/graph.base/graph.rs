//! Validate a `Manifest` and resolve it into an ordered execution plan.
//!
//! The renderer consumes a `Plan`: the pass indices split into the two `when`
//! bands (before / after window compositing), in declared order. Validation
//! catches manifest mistakes on the CPU before any GPU resource is built.

use compositor_pipeline_bundle_manifest_base::manifest::{
    requires, Manifest, Requirement, When, WindowMode,
};
use std::collections::BTreeSet;

/// Engine-provided target names, not declared in `targets`: `content` (the
/// composited scene so far), `windows` (the window layer), `output` (swapchain),
/// `history` (the previous frame's composited scene — post-composite, so
/// after-content only, like `content`; consumption-gated in the renderer).
pub const BUILTINS: [&str; 4] = ["content", "windows", "output", "history"];

/// The most images one pass may sample, counting targets, built-ins AND textures.
///
/// This is the real ceiling on a bundle's textures, and it is per PASS rather
/// than per bundle because that is where the resource actually is: every input a
/// pass names occupies one `SAMPLED_IMAGE` in its `@group(0)` descriptor set, and
/// Vulkan's guaranteed floor for `maxPerStageDescriptorSampledImages` is 16.
///
/// So a bundle may declare as many textures as its budget allows and use them
/// across as many passes as it likes — what it may not do is bind more than this
/// to any single one. Twelve leaves headroom under the floor and is past anything
/// a fullscreen pass has a reason to read at once.
pub const MAX_PASS_IMAGES: usize = 12;

/// A validated plan: pass indices per band, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Background passes, before windows are drawn.
    pub before: Vec<usize>,
    /// Overlay passes, after windows (may read `content`/`windows`).
    pub after: Vec<usize>,
}

/// Validate target references and split passes into bands. Errors on: an unknown
/// output/input target, writing an engine-produced built-in (`content`/
/// `windows`), a `before-content` pass reading a post-composite target, or no
/// pass ultimately writing `output`.
pub fn plan(m: &Manifest) -> Result<Plan, String> {
    let mut known: BTreeSet<&str> = BUILTINS.iter().copied().collect();
    for name in m.targets.keys() {
        known.insert(name.as_str());
    }
    // A texture is named in `inputs` exactly as a target is, so the two share one
    // namespace and a name in both is ambiguous. Refused rather than resolved by
    // precedence: whichever way the tie broke, half the bundles that hit it would
    // silently sample the other thing.
    for name in m.textures.keys() {
        if !known.insert(name.as_str()) {
            return Err(format!(
                "'{name}' is declared as both a texture and a target (or is an engine \
                 built-in) — `inputs` names them from one namespace, so this is ambiguous"
            ));
        }
    }
    // A texture path escaping the bundle is a bundle that can read any file the
    // compositor can. See `shader.decode::contained`.
    for (name, t) in &m.textures {
        if !compositor_pipeline_compile_decode_base::decode::contained(&t.file) {
            return Err(format!(
                "texture '{name}': `file` must be a path inside the bundle — '{}' is absolute \
                 or escapes it",
                t.file
            ));
        }
    }
    let (mut before, mut after) = (Vec::new(), Vec::new());
    let mut writes_output = false;
    for (i, p) in m.passes.iter().enumerate() {
        if matches!(p.output.as_str(), "content" | "windows" | "history") {
            return Err(format!("pass '{}': cannot write engine target '{}'", p.name, p.output));
        }
        // A texture is uploaded once and read-only thereafter; there is no path by
        // which a pass could render into one. Caught here so the author is told
        // that, rather than being told the name is not a target when it plainly is
        // declared.
        if m.textures.contains_key(p.output.as_str()) {
            return Err(format!(
                "pass '{}': output '{}' is a texture — textures are read-only inputs. Write to \
                 a `targets` entry (add `persist: true` if it must survive the frame).",
                p.name, p.output
            ));
        }
        // Every input, whatever it names, occupies one image binding in this
        // pass's descriptor set. See `MAX_PASS_IMAGES`.
        if p.inputs.len() > MAX_PASS_IMAGES {
            return Err(format!(
                "pass '{}': {} inputs — a pass may sample at most {MAX_PASS_IMAGES} images \
                 (targets and textures share the same `@group(0)` bindings). Split it into two \
                 passes through an intermediate target.",
                p.name,
                p.inputs.len(),
            ));
        }
        if !known.contains(p.output.as_str()) {
            return Err(format!("pass '{}': unknown output target '{}'", p.name, p.output));
        }
        writes_output |= p.output == "output";
        for (bind, target) in &p.inputs {
            if !known.contains(target.as_str()) {
                return Err(format!("pass '{}': input '{bind}' → unknown target '{target}'", p.name));
            }
            if p.when == When::BeforeContent
                && matches!(target.as_str(), "content" | "windows" | "history")
            {
                return Err(format!(
                    "pass '{}': before-content cannot read post-composite '{target}'",
                    p.name
                ));
            }
            // Each engine-produced image costs VRAM and a copy per frame, so it
            // is DECLARED, never inferred from the fact that a pass sampled it.
            // Inferring would put an allocation behind how a shader happens to be
            // written, which is where costs go to hide.
            if let Some(r) = Requirement::of_target(target.as_str())
                && !p.requires.contains(&r)
            {
                return Err(format!(
                    "pass '{}': samples '{target}' without declaring it — add \"{r}\" to `requires`",
                    p.name
                ));
            }
        }
        match p.when {
            When::BeforeContent => before.push(i),
            When::AfterContent => after.push(i),
        }
    }
    if !writes_output {
        return Err("no pass writes the `output` target".to_string());
    }
    // Cadence holds a target between runs, so it only makes sense for an
    // intermediate. Skipping the `output` pass would leave the frame with no
    // picture at all, not a stale one.
    for p in &m.passes {
        if p.cadence == 0 {
            return Err(format!("pass '{}': cadence must be >= 1", p.name));
        }
        if p.cadence > 1 && p.output == "output" {
            return Err(format!("pass '{}': cadence on `output` — it must run every frame", p.name));
        }
    }
    // Every pass writing the SAME target must share a cadence.
    //
    // Cadence means "hold this target between runs", and a target held by one
    // pass but rewritten every frame by another is not held at all: it alternates
    // between the two passes' results. On screen that is a per-frame flicker, in
    // proportion to however much the target contributes — which reads as
    // instability in the effect rather than as a mistake in the graph, and is
    // near-impossible to attribute by eye.
    //
    // Caught here because it is entirely static: the manifest says who writes what
    // and how often. A three-level blur pyramid whose coarsest blur was throttled
    // and whose downsample was not is exactly how this was found.
    for p in &m.passes {
        if p.output == "output" {
            continue;
        }
        if let Some(q) = m.passes.iter().find(|q| q.output == p.output && q.cadence != p.cadence) {
            return Err(format!(
                "passes '{}' (cadence {}) and '{}' (cadence {}) both write target '{}' —                  a target rewritten at two rates alternates between them, which shows as a                  per-frame flicker rather than as a held image. Give every pass writing '{}'                  the same cadence.",
                p.name, p.cadence, q.name, q.cadence, p.output, p.output,
            ));
        }
    }
    // An ownership mode takes drawables OUT of `content`, so a pass must draw them
    // back or they vanish from the desktop; and `world-*` (which widens what a
    // NON-owning bundle receives) contradicts a mode that already fixed membership
    // — under `pipeline` the shader would draw panels the engine draws too.
    if m.windows != WindowMode::Engine {
        let req = requires(m);
        if req.whole_band() {
            return Err("`world_geometry`/`world_textures` are for `windows: engine` — an \
                        ownership mode already decides what a pass receives"
                .to_string());
        }
        if !req.world_set() {
            let mode = if m.windows == WindowMode::World { "world" } else { "pipeline" };
            return Err(format!(
                "`windows: {mode}` but no pass requires `window_geometry` or \
                 `window_textures` — nothing would receive the world set"
            ));
        }
    }
    Ok(Plan { before, after })
}
