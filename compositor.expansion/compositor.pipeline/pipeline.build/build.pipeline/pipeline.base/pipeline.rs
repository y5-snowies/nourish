//! Load a multipass bundle end-to-end: parse `pipeline.json`, validate + band it
//! (`shader.graph`), compile every pass (`shader.compose` → SPIR-V), and resolve
//! each pass's input/output target references to indices — producing a
//! `CompiledPipeline` the renderer can execute directly.
//!
//! This increment handles the `before-content` band (self-contained background
//! multipass, e.g. `bloom`). `after-content` passes (which sample the built-in
//! `content`/`windows` targets) are rejected here until the renderer grows the
//! post-composite stage (Phase 2).

use compositor_pipeline_compile_compose_base::compose::{compose_wgsl, Module};
// Every bundle-relative read goes through this, so a compiled-in bundle loads by
// the same code as one on disk. See `shader.embed`.
use compositor_pipeline_bundle_embed_base::embed::{read_bytes, read_file};
use compositor_pipeline_bundle_graph_base::graph::plan;
use compositor_pipeline_bundle_manifest_base::manifest::{
    parse, requires, Format, Requires, PIPELINE_FILE, WindowMode,
};
use compositor_pipeline_bundle_property_base::{
    default_params, merge_props, parse_props, Property,
};
use compositor_pipeline_compile_spirv_base::VulkanModule;
use compositor_pipeline_abi_seam_base::base::WorldOwn;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// An intermediate offscreen target: pixel format + edge scale vs. the output.
#[derive(Clone, Copy, Debug)]
pub struct TargetSpec {
    pub format: Format,
    pub scale: f32,
    /// Double-buffered, so sampling it yields the previous frame. See
    /// `manifest::Target::persist`.
    pub persist: bool,
}

/// A declared storage buffer: its size, already rounded and capped.
#[derive(Clone, Copy, Debug)]
pub struct StorageSpec {
    pub bytes: u64,
}

/// The most one bundle may ask for, across all of its buffers.
///
/// A cap because this is the first thing in the manifest whose COST is an
/// author-chosen magnitude — every other declaration is a fixed price. 64 MiB is
/// far past any plausible accumulator and far short of a number that strands a
/// desktop, and asking for more is refused with the figure rather than trimmed,
/// because a bundle silently given less than it indexed writes off the end of
/// what it thinks it has.
pub const STORAGE_BUDGET: u64 = 64 << 20;

/// Where a pass writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// A declared intermediate target, by index into `CompiledPipeline::targets`.
    Target(usize),
    /// The swapchain (the background slot of the frame).
    Swapchain,
}

/// The built-in input sentinels, RE-EXPORTED from the executor that interprets
/// them rather than restated here.
///
/// An input index is either a target index or one of these. They used to be
/// declared independently in both crates with matching values and a comment asking
/// the reader to keep them equal — and the failure mode of them diverging is not an
/// error but a pass silently sampling the wrong built-in image. Nothing would have
/// caught it: the values are plausible either way.
///
/// * `CONTENT` — the composited scene. After-content passes only.
/// * `HISTORY` — the previous frame's composited scene. Readable in either band
///   (it predates this frame); consumption-gated on `previous_frame`.
/// * `WINDOWS` — the composited window layer: client windows over transparency,
///   background excluded. This is what makes window content reachable from a pass
///   without touching client buffers — the compositor has already resolved dmabuf
///   and SHM surfaces into one image, so a shader reads both the same way and
///   nothing crosses a device. Consumption-gated on `window_layer`.
///
/// `TEXTURE` is the base of a fourth family rather than a single value: an input
/// index of `TEXTURE + i` samples the bundle's i-th declared texture. Same
/// re-export rule and the same reason.
pub use compositor_pipeline_execute_graph_base::graph::{
    CONTENT, HISTORY, TEXTURE, WINDOWS,
};

/// One compiled pass ready to run.
pub struct CompiledPass {
    pub name: String,
    pub module: VulkanModule,
    /// Sampled inputs, by index, in binding order (sorted by the pass's input
    /// binding name — matches the `EffectPass` binding ABI). `CONTENT` = the
    /// composited scene.
    pub inputs: Vec<usize>,
    pub output: Output,
    /// Default `@prop` params (slot i = prop #i) for this pass's shader.
    pub params: [f32; 16],
    /// For each of this pass's own param slots, that prop's index in the BUNDLE
    /// union (`CompiledPipeline::properties`).
    ///
    /// The two orders differ — a pass's array follows that pass's declaration
    /// order, the union follows manifest pass order — so a live edit without this
    /// map lands in the wrong pass slot. Resolved once, so per frame it is a lookup.
    pub union: Vec<usize>,
    /// What this pass requires bound. `world_set()` binds the geometry UBO at
    /// `@group(1) @binding(0)`; `textures()` also binds the bindless array at
    /// `@binding(1)` and needs the device's descriptor-indexing feature.
    pub requires: Requires,
    /// Run every Nth frame, holding the target in between (1 = every frame).
    pub cadence: u32,
}

/// A bundle compiled to an executable multipass pipeline.
pub struct CompiledPipeline {
    pub id: u64,
    pub targets: Vec<TargetSpec>,
    /// Declared storage buffers, in sorted-name order — which IS their binding
    /// order at `@group(2)`. Empty for every bundle that declares none, which is
    /// what keeps the descriptor set from being built at all.
    pub storage: Vec<StorageSpec>,
    /// Declared textures, decoded, in sorted-name order — which IS the order the
    /// `TEXTURE` sentinel indexes them by. Empty for every bundle that declares
    /// none, which is what keeps the decode, the staging and the upload from
    /// happening at all.
    pub textures: Vec<compositor_pipeline_compile_decode_base::decode::Decoded>,
    /// Passes in `before-content` execution order.
    pub before: Vec<CompiledPass>,
    /// After-content passes (sample the composited `content`), in order.
    pub after: Vec<CompiledPass>,
    /// Union of every pass's `@prop`s (first definition wins), for the settings UI.
    pub properties: Vec<Property>,
    /// The union of every pass's `requires` — THE gate for every engine cost this
    /// bundle incurs, and the only reason the engine does anything extra for it.
    /// Empty means this bundle costs what a single-pass shader costs.
    pub requires: Requires,
    /// How much of the world band the compositor leaves OUT of `content` for
    /// this pipeline to composite itself (§8d). `Windows` is client windows only
    /// and leaves iced-world panels engine-drawn (hence on top); `World` is the
    /// whole band, in draw order.
    pub owns: compositor_pipeline_abi_seam_base::base::WorldOwn,
    /// How much of this graph the background worker could take, decided once at
    /// load from the manifest (`shader.place`). Carried so the producer does not
    /// re-derive it every frame.
    pub offload: compositor_pipeline_build_place_base::place::Offload,
    /// Why any pass lost the worker without having asked for it. Advisory; logged.
    pub place_notes: Vec<String>,
    /// Why a pass that explicitly asked for the worker is not getting it. Non-empty
    /// means the producer REFUSES this bundle: a shader cannot dictate something it
    /// does not get, and running it in a shape its author neither chose nor can see
    /// is how the failure stayed invisible.
    pub place_denied: Vec<String>,
    /// This bundle's pointer warp, parsed and validated at load. `None` when the
    /// bundle displaces nothing, which is the common case and costs nothing.
    pub warp: Option<std::sync::Arc<compositor_pipeline_host_hit_base::hit::Warp>>,
    /// Where the values `hit.params` NAMED live in the bundle union — one slot per
    /// argument lane, `usize::MAX` for the lanes the bundle did not name.
    ///
    /// Slots rather than values, so the warp reads the same live array the passes
    /// are pushed from — a snapshot of the defaults diverges the moment a slider
    /// moves. See `draw.select::warp_values`.
    pub warp_slots: [usize; 4],
    /// How to serve the griddable entry — see `manifest::Evaluate`.
    pub warp_evaluate: compositor_pipeline_bundle_manifest_base::manifest::Evaluate,
    /// The generated fullscreen pass that renders the warp grid on the GPU, for
    /// `evaluate: "map"`. Composed from the bundle's OWN warp module, so the map
    /// and the picture cannot be two different functions.
    pub warp_map_module: Option<VulkanModule>,
    /// This bundle's window-chrome policy, carried through from the manifest and
    /// published when the bundle is selected. See `bridge.window::chrome`.
    pub decorations: compositor_pipeline_bundle_manifest_base::manifest::Decorations,
    pub letterbox: compositor_pipeline_bundle_manifest_base::manifest::LetterboxMode,
}

impl CompiledPipeline {
    /// Whether a frame carrying this bundle is composed WHOLE rather than only
    /// where something changed, and so needs the whole element list.
    ///
    /// `submit_frame` already forces a full clear for an offscreen graph and an
    /// owned band, but `ops` holds only what the damage tracker chose to draw — so
    /// the clear erases everything that did not change.
    pub fn composes_whole_frame(&self) -> bool {
        !self.requires.is_empty()
            || self.owns != compositor_pipeline_abi_seam_base::base::WorldOwn::Engine
    }
}

/// The `#define_import_path` a warp module declares, so the generated pass can
/// `#import` its `hit_inverse` rather than duplicating the maths.
fn import_path(src: &str) -> Option<String> {
    src.lines()
        .find_map(|l| l.trim().strip_prefix("#define_import_path"))
        .map(|r| r.trim().to_string())
}

/// Whether the module's `hit_inverse` takes the optional clock argument. Read off
/// the source rather than the parsed module because the generated pass has to be
/// written before anything is compiled.
pub fn animated_source(src: &str) -> bool {
    src.lines()
        .find(|l| l.contains("fn hit_inverse"))
        .is_some_and(|l| l.matches(',').count() >= 3)
}

/// The generated grid pass: evaluate the bundle's own `hit_inverse` per cell and
/// write the source UV out. `@location(0)` is `R32G32_SFLOAT`, so the two lanes
/// land as the two floats `warpmap` copies back.
///
/// The fragment's own coordinate is the GRID cell, but `res` must stay the OUTPUT
/// resolution — the warp is aspect-corrected by it, and correcting by 128×128 would
/// make the curve round on a square that is not the screen.
///
/// `uv` is `(frag.xy - 0.5) / (EDGE - 1)`, NOT `frag.xy / EDGE`. A fragment centre
/// sits at `px + 0.5`, and `Map::sample` places node `x` at `x / (EDGE - 1)` — so
/// the naive form stores each cell half a texel off and scaled by `EDGE/(EDGE-1)`,
/// which reads as a pointer that drifts further the further it is from centre. The
/// CPU baker and this must land on the same nodes or the two producers are two
/// different maps.
pub fn warp_pass_src(path: &str, animated: bool) -> String {
    let call = if animated {
        "hit_inverse(uv, res, pc.params[0], pc.res_zoom_time.w)"
    } else {
        "hit_inverse(uv, res, pc.params[0])"
    };
    format!(
        "#import {path}::hit_inverse\n\
         struct Push {{\n\
         \x20   res_zoom_time: vec4<f32>,\n\
         \x20   pan_flow: vec4<f32>,\n\
         \x20   lock_alpha: vec4<f32>,\n\
         \x20   params: array<vec4<f32>, 2>,\n\
         }};\n\
         var<immediate> pc: Push;\n\
         @fragment\n\
         fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {{\n\
         \x20   let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));\n\
         \x20   let uv = (frag.xy - vec2<f32>(0.5)) / vec2<f32>({edge}.0 - 1.0);\n\
         \x20   let w = {call};\n\
         \x20   return vec4<f32>(w, 0.0, 1.0);\n\
         }}\n",
        edge = compositor_pipeline_execute_warpmap_base::base::EDGE
    )
}

/// A hex digest of arbitrary bytes, so non-text bundle inputs can join the string
/// parts that make up the cache key.
fn digest(bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn hash_of(parts: &[&str]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    h.finish()
}

/// Load + compile the multipass bundle at `bundle` (its directory). `Err` on any
/// missing/invalid file so the caller can fall back to the single-pass path.
pub fn load_pipeline(
    bundle: &Path,
    env: compositor_pipeline_build_place_base::place::Env,
) -> Result<CompiledPipeline, String> {
    let manifest_src = read_file(bundle, PIPELINE_FILE)?;
    let manifest = parse(&manifest_src)?;
    let planned = plan(&manifest)?;
    let placement = compositor_pipeline_build_place_base::place::place(&manifest, &planned, env);

    // Stable target index by sorted name (BTreeMap iterates sorted).
    let target_index: BTreeMap<&str, usize> =
        manifest.targets.keys().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    // Storage: rounded to the `vec4` stride and budgeted as a whole, so a bundle
    // cannot slip past the cap by declaring twenty small buffers.
    let storage: Vec<StorageSpec> = manifest
        .storage
        .values()
        .map(|s| StorageSpec { bytes: s.bytes.max(16).next_multiple_of(16) })
        .collect();
    let asked: u64 = storage.iter().map(|s| s.bytes).sum();
    if asked > STORAGE_BUDGET {
        return Err(format!(
            "storage: this bundle asks for {asked} bytes across {} buffers; the budget is \
             {STORAGE_BUDGET}. Refused rather than trimmed — a buffer smaller than the one \
             the shader indexes writes off the end of it.",
            storage.len(),
        ));
    }
    let targets: Vec<TargetSpec> = manifest
        .targets
        .values()
        .map(|t| TargetSpec { format: t.format, scale: t.scale, persist: t.persist })
        .collect();

    // Textures: read, decoded and budgeted HERE, so a bundle whose art is missing
    // or oversized fails to load like a bundle whose WGSL does not compile. The
    // alternative — a lazy upload at first use — turns a mistake in the manifest
    // into a black patch on the desktop several frames later, with the pass that
    // sampled it looking like the culprit.
    //
    // Sorted-name order (BTreeMap) is the index space the `TEXTURE` sentinel uses,
    // matching how targets are indexed and for the same reason: it is derivable
    // from the manifest without carrying a second list.
    let texture_index: BTreeMap<&str, usize> =
        manifest.textures.keys().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let mut texture_bytes: Vec<Vec<u8>> = Vec::new();
    let mut textures: Vec<compositor_pipeline_compile_decode_base::decode::Decoded> = Vec::new();
    for (name, t) in &manifest.textures {
        let raw = read_bytes(bundle, &t.file)?;
        textures.push(compositor_pipeline_compile_decode_base::decode::decode(
            name, &raw, t.srgb,
        )?);
        texture_bytes.push(raw);
    }
    compositor_pipeline_compile_decode_base::decode::budget(&textures)?;

    // Read the shared `#import` modules once.
    let module_srcs: Vec<(String, String)> = manifest
        .modules
        .iter()
        .map(|rel| {
            read_file(bundle, rel).map(|s| (s, rel.clone()))
        })
        .collect::<Result<_, _>>()?;
    let modules: Vec<Module> =
        module_srcs.iter().map(|(s, p)| Module { source: s, file_path: p }).collect();

    let bundle_path = bundle.to_string_lossy().into_owned();

    // Every pass source, read ONCE, indexed by manifest pass index.
    let pass_srcs: Vec<String> = manifest
        .passes
        .iter()
        .map(|p| {
            read_file(bundle, &p.shader)
        })
        .collect::<Result<_, _>>()?;

    // The cache key identifies a COMPILED pipeline, so it is derived from the
    // sources and not only from where they live.
    //
    // The renderer caches each pass's Vulkan pipeline by this id and `GraphExec`
    // rebuilds nothing while the id is unchanged. Keyed on the bundle path alone,
    // re-loading an edited bundle produced fresh SPIR-V that the renderer then
    // ignored in favour of what it had already compiled from the same path — so
    // an edit had no effect on screen and nothing anywhere said why. That is the
    // whole of the edit-reload loop the `Shader` gRPC service exists to serve.
    //
    // Content, not mtime: an editor that rewrites a file byte-identically should
    // not force a recompile of every pass, and a filesystem without reliable
    // timestamps should not stop one that is needed.
    //
    // TEXTURES COUNT AS SOURCE. They are read by the same reload and they change
    // the picture the same way, but they are not text, so they enter as a digest
    // per file. Left out, editing a sprite sheet and reloading produced an
    // identical id, `GraphExec::prepare` saw `changed == false`, nothing
    // reallocated, and the new art never reached the GPU — the same failure
    // `tests/reload.rs` was written about, one input later.
    let texture_digests: Vec<String> = texture_bytes.iter().map(|b| digest(b)).collect();
    let bundle_key = {
        let mut parts: Vec<&str> = vec![&*bundle_path];
        parts.extend(pass_srcs.iter().map(String::as_str));
        parts.extend(module_srcs.iter().map(|(s, _)| s.as_str()));
        parts.extend(texture_digests.iter().map(String::as_str));
        format!("{}#{:016x}", bundle_path, hash_of(&parts))
    };
    // The bundle's variable union, in MANIFEST order — not band order.
    //
    // Band order is `before` then `after`, which for a bundle whose manifest
    // interleaves the two is a different permutation. The union index is the param
    // slot the settings panel edits, and `shader.load::properties_for` builds the
    // same list from the manifest alone (for a bundle that is not loaded yet), so
    // the two have to agree on ORDER and not merely on membership.
    let mut properties: Vec<Property> = Vec::new();
    for src in &pass_srcs {
        merge_props(&mut properties, parse_props(src));
    }
    let compile = |pi: usize| -> Result<CompiledPass, String> {
        let pass = &manifest.passes[pi];
        let src = &pass_srcs[pi];
        let id = hash_of(&[&bundle_key, &pass.name]);
        let module = compose_wgsl(src, &pass.shader, &modules, &pass.defines, id)?;

        // Resolve inputs (binding order = sorted binding name): a declared target
        // index, or the built-in `content` sentinel (after-content passes only).
        let mut inputs = Vec::new();
        for (bind, target) in &pass.inputs {
            let idx = if target == "content" {
                CONTENT
            } else if target == "windows" {
                WINDOWS
            } else if target == "history" {
                HISTORY
            } else if let Some(ti) = texture_index.get(target.as_str()) {
                // A texture binds exactly where a target would, in the same
                // sorted-binding-name order, so the shader cannot tell them apart
                // and nothing downstream has a second list to keep in step.
                TEXTURE + ti
            } else {
                *target_index.get(target.as_str()).ok_or_else(|| {
                    format!(
                        "pass '{}': input '{bind}' → '{target}' is not a target or texture",
                        pass.name
                    )
                })?
            };
            inputs.push(idx);
        }
        let output = if pass.output == "output" {
            Output::Swapchain
        } else {
            Output::Target(*target_index.get(pass.output.as_str()).ok_or_else(|| {
                format!("pass '{}': unknown output target '{}'", pass.name, pass.output)
            })?)
        };

        // This pass's own props, and where each of them sits in the bundle union.
        // Every name is present by construction — the union was folded from these
        // very sources above — so a miss can only mean the two loops disagreed,
        // and `usize::MAX` makes that inert rather than silently mis-slotted.
        let props = parse_props(src);
        let params = default_params(&props);
        let union = props
            .iter()
            .map(|p| properties.iter().position(|q| q.name == p.name).unwrap_or(usize::MAX))
            .collect();
        Ok(CompiledPass {
            name: pass.name.clone(),
            module,
            inputs,
            output,
            params,
            union,
            requires: Requires::of(pass.requires.iter().copied()),
            cadence: pass.cadence.max(1),
        })
    };

    let mut before = Vec::new();
    for &pi in &planned.before {
        before.push(compile(pi)?);
    }
    let mut after = Vec::new();
    for &pi in &planned.after {
        after.push(compile(pi)?);
    }

    // The one place a bundle's total cost is decided. `plan()` has already refused
    // a pass that samples a built-in image without declaring it, so this union is
    // complete — there is no path by which the engine allocates something no entry
    // here asked for.
    let requires = requires(&manifest);
    // The pointer warp. Parsed HERE, not lazily at first use: a bundle whose warp
    // cannot be evaluated must fail to load like any other broken source, rather
    // than render correctly while the cursor quietly lands somewhere else.
    let warp = match &manifest.hit {
        Some(h) => {
            let src = read_file(bundle, &h.module)?;
            Some(std::sync::Arc::new(compositor_pipeline_host_hit_base::hit::parse(&src)?))
        }
        None => None,
    };
    // `evaluate: map` renders the grid on the GPU each frame. Compile the pass for
    // it HERE, from the bundle's own warp module, so the map and the picture can
    // never be two different functions.
    use compositor_pipeline_bundle_manifest_base::manifest::Evaluate;
    let warp_evaluate = manifest.hit.as_ref().map(|h| h.evaluate).unwrap_or_default();
    let warp_map_module = match (&manifest.hit, warp_evaluate) {
        (Some(h), Evaluate::Map) => {
            let src = read_file(bundle, &h.module)?;
            let path = import_path(&src).ok_or_else(|| {
                format!(
                    "hit: `evaluate: \"map\"` needs `{}` to declare `#define_import_path`, \
                     so the generated pass can import its `hit_inverse`",
                    h.module
                )
            })?;
            let wrapper = warp_pass_src(&path, animated_source(&src));
            Some(compose_wgsl(
                &wrapper,
                "generated/warpmap.wgsl",
                &modules,
                &BTreeMap::new(),
                hash_of(&[&bundle_key, "warpmap"]),
            )?)
        }
        _ => None,
    };
    // A static bake of a warp that reads the clock describes one instant and is
    // wrong every frame after it. Refuse rather than serve something stale.
    if warp_evaluate == Evaluate::MapStatic
        && warp.as_ref().is_some_and(|w| w.animated())
    {
        return Err("hit: this warp reads the clock, so a static bake describes one \
                    instant — declare `evaluate: \"map\"` (GPU, per frame) or \
                    `\"pointwise\"`"
            .to_string());
    }
    // A per-drawable warp iterates the world set, so the bundle must ask for it.
    // Without the requirement the set is never collected, the loop sees nothing,
    // and the warp silently does nothing at all — the failure mode this whole
    // declaration surface exists to make impossible.
    if let Some(w) = &warp
        && w.has_drawable()
        && !requires.world_set()
    {
        return Err(format!(
            "hit: the module defines `{}`, which reads the world set — the bundle \
             must also require `window_geometry` or `world_geometry`",
            compositor_pipeline_host_hit_base::hit::ENTRY_DRAWABLE
        ));
    }
    // Resolve the warp's named props to their SLOTS in the union, AFTER every
    // pass has contributed. An unknown name is an error, not a zero: a warp
    // silently driven by 0.0 is a pointer that is subtly and inexplicably wrong.
    //
    // Slots, not values. Baking the declared defaults in here made the warp a
    // DIFFERENT FUNCTION from the picture the moment anyone moved the slider:
    // the pass gets live values (through `CompiledPass::union`), so a bundle
    // whose `curve` had been edited drew one curvature and corrected the pointer
    // for another. It agreed only for as long as nothing was ever tuned, which is
    // the worst kind of agreement — it holds in every test and breaks on first use.
    let mut warp_slots = [usize::MAX; 4];
    if let Some(h) = &manifest.hit {
        for (i, name) in h.params.iter().take(4).enumerate() {
            warp_slots[i] = properties.iter().position(|p| &p.name == name).ok_or_else(|| {
                format!("hit: `params` names `{name}`, which no pass declares as an @prop")
            })?;
        }
    }
    Ok(CompiledPipeline {
        id: hash_of(&[&bundle_key]),
        targets,
        storage,
        textures,
        before,
        after,
        properties,
        requires,
        owns: match manifest.windows {
            WindowMode::Engine => WorldOwn::Engine,
            WindowMode::Pipeline => WorldOwn::Windows,
            WindowMode::World => WorldOwn::World,
        },
        warp,
        warp_slots,
        warp_evaluate,
        warp_map_module,
        offload: placement.offload,
        place_notes: placement.notes,
        place_denied: placement.denied,
        decorations: manifest.decorations,
        letterbox: manifest.letterbox,
    })
}
