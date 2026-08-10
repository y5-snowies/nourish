//! Build the renderer seam for a compiled pipeline, and answer the pointer
//! against the warp that pipeline declares.
//!
//! Split out of `background.two/two.draw/draw.select`, which did two unrelated
//! jobs: deciding which background shader a world has SELECTED (a background
//! concern, and still there), and turning the resulting `CompiledPipeline` into
//! something the renderer can run (pipeline machinery, and here).
//!
//! The split is the rule the pipeline root exists to enforce — the background
//! may use a pipeline but must not define how one works. `draw.select` now calls
//! into this; nothing here knows what a parallax is.

use compositor_pipeline_build_pipeline_base::pipeline::{CompiledPipeline, Output as PassOutput};
use compositor_orchestration_draw_dispatch_frame::{NativeShaderPass, ParallaxUniforms, ShaderVariant};
use compositor_pipeline_abi_seam_base::base::{PipelineOutput, PipelinePass, PipelineTarget, PipelineTexture, ShaderPipeline, TargetFormat};
use std::borrow::Cow;

/// The values the warp's named `@prop`s hold RIGHT NOW, from the live array.
///
/// The warp and the picture must be one function, values included: the pass is
/// pushed from `live`, so resolving the warp from the compile-time defaults would
/// correct the cursor for a curvature that is no longer on screen. Unnamed lanes
/// stay 0.0.
pub fn warp_values(cp: &CompiledPipeline, live: &[f32; 16]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for (i, &slot) in cp.warp_slots.iter().enumerate() {
        if let Some(v) = live.get(slot) {
            out[i] = *v;
        }
    }
    out
}

/// Apply `cp`'s pointer warp to one point — the CHAIN of the entries it declared.
///
/// A function over the world's own state, not a published closure. The warp
/// belongs to whichever bundle THIS world is running, so it is read from the
/// world's `Two` slot at the point of use; a process-global closure could only
/// ever describe one world's bundle, and quietly described the last one selected.
///
/// The chain is applied in reverse of how the picture was built — LAST pass undone
/// first. A bundle composites its drawables (each with its own transform) into a
/// band, then displaces that band as a whole; so the pointer removes the
/// fullscreen displacement first, landing in band space, and only then tests the
/// drawable rects, which are in band space. The other order tests those rects
/// against a point the global warp has not been taken out of yet, and is wrong by
/// exactly the curvature.
///
/// `cache` is this world's baked map for `evaluate: "map_static"` — per world for
/// the same reason, and `&mut` because a stale one is rebaked in place.
///
/// Anything returning `None` — an unsupported construct, the step budget, a bake
/// that failed — falls back to identity for that point: briefly uncorrected beats
/// arbitrarily wrong.
///
/// `live` is the element's current param array — the same one the passes are
/// pushed from. See [`warp_values`].
pub fn warp_point(
    cp: &CompiledPipeline,
    live: &[f32; 16],
    cache: &mut Option<compositor_pipeline_host_map_base::map::Map>,
    set: Option<&compositor_pipeline_abi_worldset_base::base::WorldSet>,
    grid: Option<&[[f32; 2]]>,
    u: f64,
    v: f64,
    res: [f32; 2],
) -> (f64, f64) {
    use compositor_pipeline_host_hit_base::hit::Args;
    use compositor_pipeline_bundle_manifest_base::manifest::Evaluate;
    let Some(w) = cp.warp.as_ref() else { return (u, v) };
    let params = warp_values(cp, live);
    let (map, drawable) = (w.has_map(), w.has_drawable());
    let baked_map = map && cp.warp_evaluate == Evaluate::MapStatic;
    let gpu_map = map && cp.warp_evaluate == Evaluate::Map;
    // The clock, read HERE rather than taken from a slot the background element
    // filled on its last draw.
    //
    // It used to be `warp::frame_time()`, an atomic written from
    // `ParallaxBackground::draw()`. That is wrong twice over: `draw()` runs once
    // per PANE, so the last pane drawn won; and a FULLY OFFLOADED bundle never
    // reaches `draw()` at all, so the slot froze at whatever the last inline
    // frame left and every pointer event was un-warped with a stale clock — the
    // cursor drifting further from the picture the longer the bundle ran.
    //
    // A pointer event is not a frame and does not want a frame's timestamp: it
    // wants the clock now. Both the shader and this read the same `window.clock`
    // origin, so "the same clock" is preserved without a slot to keep in step.
    let t = compositor_pipeline_abi_clock_base::base::now();
    let args = Args { res, params, time: t };
    let (mut u, mut v) = (u, v);

    // 1. The fullscreen displacement — the LAST pass, so the first to undo. Lands
    //    the point in band space.
    if gpu_map {
        // Produced on the GPU, one frame old. Sampled through the `Arc` in place:
        // wrapping it in a `Map` copied the whole 128 KB grid per motion event.
        match grid {
            Some(g) if g.len() == compositor_pipeline_host_map_base::map::EDGE.pow(2) => {
                (u, v) = compositor_pipeline_host_map_base::map::sample_cells(g, u, v);
            }
            // No grid yet — the producer has published none, or the bundle just
            // changed. Evaluating directly keeps the pointer correct rather than
            // briefly flat.
            _ => (u, v) = w.eval((u, v), args).unwrap_or((u, v)),
        }
    } else if map && !baked_map {
        // Served pointwise by request: exact, no bake, no staleness.
        (u, v) = w.eval((u, v), args).unwrap_or((u, v));
    } else if map {
        match cache.as_ref().filter(|m| m.matches(res, params)) {
            Some(m) => (u, v) = m.sample(u, v),
            None => {
                // Stale or unbaked: answer THIS event by evaluating directly, so
                // there is no window in which the pointer is uncorrected while a
                // map is being built, then bake for the events after it.
                (u, v) = w.eval((u, v), args).unwrap_or((u, v));
                *cache = compositor_pipeline_host_map_base::map::Map::bake(w, res, params);
            }
        }
    }

    // 2. Per-drawable, FRONT to back — the published set is back-to-front, so
    //    iterate it reversed. The first drawable to CLAIM the point owns it, which
    //    is what makes the topmost window win; the engine owns this iteration so no
    //    bundle re-implements z order.
    if drawable {
        // The drawn world's own drawables, handed in by the seat — the same value
        // the shader was fed, not a second read of a shared slot that another
        // world may have written since.
        let Some(set) = set else { return (u, v) };
        for i in (0..set.rects.len()).rev() {
            // Mirrors the shader's own `windows.attrs[i]`, plus the index and clock
            // the pass looped with — a transform built from those cannot be undone
            // without them. Named `attrs`, not `meta`: `meta` is a reserved WGSL
            // keyword.
            //
            // The descriptors go in SEPARATELY rather than displacing a lane here:
            // this vector's four are documented and implemented by a shipped
            // example, so repointing one would change what an existing warp reads
            // without changing the warp. Same value the render pass gets from
            // `windows.attrs[i].z`, so the two cannot disagree about the window.
            let attrs = [i as f32, t, set.kinds[i] as u8 as f32, set.alphas[i]];
            let flags = set.flags.get(i).copied().unwrap_or(0);
            if let Some(p) = w.eval_drawable((u, v), args, set.rects[i], attrs, flags) {
                (u, v) = p;
                break;
            }
        }
    }
    (u, v)
}

/// Whether `cp` displaces the pointer at all — cheap enough to ask per event.
pub fn warps(cp: &CompiledPipeline) -> bool {
    cp.warp.as_ref().is_some_and(|w| w.has_map() || w.has_drawable())
}

fn seam_format(f: compositor_pipeline_bundle_manifest_base::manifest::Format) -> TargetFormat {
    use compositor_pipeline_bundle_manifest_base::manifest::Format;
    match f {
        Format::Rgba8 => TargetFormat::Rgba8,
        Format::Rgba16f => TargetFormat::Rgba16f,
        Format::Rgba32f => TargetFormat::Rgba32f,
    }
}

/// Build the dispatch-seam pass for a multipass pipeline: each pass's SPIR-V +
/// per-pass engine push (packed from the live uniforms + that pass's params),
/// with target/input/output wiring. The renderer runs the graph.
///
/// `live` is the BUNDLE-union array; each pass's own array is indexed differently,
/// so it is rebuilt through `CompiledPass::union`. Packing `c.params` alone is
/// what made a multipass bundle ignore every edited variable.
pub fn multipass_pass<'a>(
    cp: &'a CompiledPipeline,
    u: &ParallaxUniforms,
    live: &[f32; 16],
) -> NativeShaderPass {
    let variant = |c: &'a compositor_pipeline_build_pipeline_base::pipeline::CompiledPass| {
        // Defaults for anything the union does not reach (a prop past slot 16, or
        // the `usize::MAX` miss), live values for everything it does.
        let mut params = c.params;
        for (slot, &ux) in c.union.iter().enumerate().take(16) {
            if let Some(v) = live.get(ux) {
                params[slot] = *v;
            }
        }
        ShaderVariant {
            id: c.module.id,
            // `Arc`, not `Cow`: the seam is crossed every frame but the renderer
            // reads the bytes only on a pipeline-cache miss, so this is a refcount
            // bump rather than a copy.
            spv: c.module.spv.clone(),
            vert_spv: c.module.vert_spv.clone(),
            vert_entry: c.module.vert_entry.clone(),
            frag_entry: c.module.frag_entry.clone(),
            push: std::sync::Arc::from(
                compositor_pipeline_abi_push_base::base::engine_push(u, &params).to_vec(),
            ),
        }
    };
    let mk = |list: &'a [compositor_pipeline_build_pipeline_base::pipeline::CompiledPass]| {
        list.iter()
            .map(|c| PipelinePass {
                variant: variant(c),
                inputs: c.inputs.clone(),
                output: match c.output {
                    PassOutput::Target(i) => PipelineOutput::Target(i),
                    PassOutput::Swapchain => PipelineOutput::Swapchain,
                },
                requires: c.requires,
                cadence: c.cadence,
            })
            .collect::<Vec<_>>()
    };
    let targets = cp
        .targets
        .iter()
        .map(|t| PipelineTarget { format: seam_format(t.format), scale: t.scale, persist: t.persist })
        .collect();
    NativeShaderPass {
        // `sdr` is unused when `pipeline` is set, but the struct requires it; any
        // pass is a safe stand-in. An after-content-only bundle (e.g. `vignette`)
        // has no before passes, so fall back to the after band.
        sdr: variant(
            cp.before
                .last()
                .or_else(|| cp.after.last())
                .expect("pipeline has ≥1 pass"),
        ),
        hdr: None,
        pipeline: Some(std::sync::Arc::new(ShaderPipeline {
            id: cp.id,
            targets,
            storage: cp.storage.iter().map(|s| s.bytes).collect(),
            // Decoded once at load and refcounted across the seam from here on —
            // the worker builds this same seam from the same `CompiledPipeline`,
            // so both devices upload the identical bytes without decoding twice.
            textures: cp
                .textures
                .iter()
                .map(|t| PipelineTexture {
                    width: t.width,
                    height: t.height,
                    srgb: t.srgb,
                    pixels: t.pixels.clone(),
                })
                .collect(),
            passes: mk(&cp.before),
            after: mk(&cp.after),
            requires: cp.requires,
            owns: cp.owns,
            warp_map: cp.warp_map_module.as_ref().map(|m| ShaderVariant {
                id: m.id,
                spv: m.spv.clone(),
                vert_spv: m.vert_spv.clone(),
                vert_entry: m.vert_entry.clone(),
                frag_entry: m.frag_entry.clone(),
                // The values `hit.params` NAMED, from the LIVE array. A pass's own
                // array is indexed by that pass's declaration order, so feeding one
                // here made `crt-input-map-live` read the `world` pass's `tilt` as
                // its curvature.
                //
                // `res` and the clock ride the same engine push the picture gets,
                // so the grid is the same function at the same instant.
                push: std::sync::Arc::from({
                    let mut params = [0.0f32; 16];
                    params[..4].copy_from_slice(&warp_values(cp, live));
                    compositor_pipeline_abi_push_base::base::engine_push(u, &params).to_vec()
                }),
            }),
        })),
    }
}

