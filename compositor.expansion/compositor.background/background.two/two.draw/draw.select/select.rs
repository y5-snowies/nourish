use compositor_pipeline_build_pipeline_base::pipeline::{CompiledPipeline, Output as PassOutput};
use compositor_pipeline_compile_spirv_base::VulkanModule;
use compositor_orchestration_draw_dispatch_frame::{NativeShaderPass, ParallaxUniforms, ShaderVariant};
use compositor_pipeline_abi_seam_base::base::{PipelineOutput, PipelinePass, PipelineTarget, PipelineTexture, ShaderPipeline, TargetFormat};
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesRenderer};
use std::borrow::Cow;
use std::sync::Arc;

/// Load the multipass pipeline for `selection` if the bundle carries a
/// `pipeline.json` (Vulkan only). `None` → the single-pass path runs.
///
/// Returns the REASON alongside it, the same shape `shader.load::load` returns, so
/// a refusal reaches the settings card instead of only a log line. That mattered
/// more here than there: a `passes/`-only bundle has no single-pass source, so the
/// old loader found nothing to complain about and the user got the stock parallax
/// behind a clean, error-free panel.
///
/// Falling back is never fatal — the desktop always has a background.
pub fn load_multipass(
    selection: Option<&str>,
) -> (Option<Arc<CompiledPipeline>>, Option<String>) {
    // NOTHING about the engine's behaviour is published here.
    //
    // Chrome and the warp were already read from this world's `Two` slot. The
    // requirement set, the band ownership and the whole-frame flag used to be
    // published here, once per selection — and that is precisely the multi-world
    // bug: this function runs only when a world BUILDS an instance, so a world
    // returning to the screen with an already-compiled bundle never re-published,
    // and the engine kept following whichever world built last.
    //
    // They are now stated per frame by the world being drawn.
    //
    // A selection-time floor was kept here for a while alongside that, to cover
    // the frames before whichever per-frame site next ran. What retired it is
    // that ALL THREE prepare paths now publish, not just the orchestration
    // scene's: the frame driver picks one prepare per frame, and the picker and
    // the lock screen build their own, so on their frames nothing restated the
    // facts and the globals kept whatever the last ordinary frame left behind.
    //
    // No symptom was ever pinned on that gap, and none is claimed here — the
    // objection is structural. A floor publishes the facts of the world doing the
    // SELECTING, which is the same "some other world's answers" shape as the bug
    // above, and it is only ever corrected by the per-frame statement it was
    // supposed to be backing up. With every drawing path stating its own facts
    // there is nothing left for it to floor.
    //
    // See `pipeline.build/build.seam::publish_world_facts`.
    let Some(sel) = selection else { return (None, None) };
    let bundle = compositor_pipeline_bundle_locate_base::resolve_ref(sel);
    if !compositor_pipeline_bundle_embed_base::embed::has_manifest(&bundle) {
        // Not a multipass bundle at all — the single-pass path handles it, and any
        // error there is its own to report.
        return (None, None);
    }
    // What this session can actually offer, resolved once and handed to placement
    // so its verdict describes what will HAPPEN rather than what the manifest
    // hoped for.
    let env = compositor_pipeline_build_place_base::place::Env {
        worker: compositor_model_environment_background_base::base::get().engaged(),
    };
    match compositor_pipeline_build_pipeline_base::pipeline::load_pipeline(&bundle, env) {
        Ok(cp) => {
            // The bindless array needs the device's descriptor-indexing feature.
            // That is a property of the GPU, probed once at renderer init and not
            // influenced by which bundle is selected — so a device without it can
            // never run this bundle, and saying so is the whole message.
            if cp.requires.textures()
                && !compositor_model_stats_registry_base::base::descriptor_indexing()
            {
                let why = "requires the window-texture array, which this GPU cannot \
                           provide (no descriptor indexing)"
                    .to_string();
                warn!("background.pipeline: {sel} {why}");
                return (None, Some(why));
            }
            // A pass asked for something it is not getting. Refuse the bundle
            // rather than run it in a shape its author neither chose nor can see:
            // it ran inline anyway before, behind one `warn!`, while the load line
            // in the same breath reported the offload it was not doing.
            // Writing a storage buffer from a fragment shader is a device
            // FEATURE, probed once at renderer init like descriptor indexing —
            // so a device without it can never run this bundle, and the shader
            // would not error, it would simply have its writes discarded.
            if !cp.storage.is_empty()
                && !compositor_model_stats_registry_base::base::storage_writes()
            {
                let why = "declares `storage`, which this GPU cannot write from a \
                           fragment shader (no fragmentStoresAndAtomics)"
                    .to_string();
                warn!("background.pipeline: {sel} {why}");
                return (None, Some(why));
            }
            if !cp.place_denied.is_empty() {
                let why = cp.place_denied.join("; ");
                error!("background.pipeline: {sel} refused — {why}");
                return (None, Some(why));
            }
            for n in &cp.place_notes {
                warn!("background.pipeline: {sel}: {n}");
            }
            info!(
                "background.pipeline: loaded {sel} ({} passes, {} targets, offload={:?}, \
                 requires={:?})",
                cp.before.len(),
                cp.targets.len(),
                cp.offload,
                cp.requires
            );
            if compositor_pipeline_build_seam_base::base::warps(&cp) {
                let w = cp.warp.as_ref().expect("warps() implies a parsed warp");
                info!(
                    "background.pipeline: pointer warp active — evaluate={:?}, \
                     per-drawable={}, animated={}",
                    cp.warp_evaluate,
                    w.has_drawable(),
                    w.animated()
                );
            }
            announce_chrome(&cp);
            (Some(Arc::new(cp)), None)
        }
        Err(e) => {
            error!("background.pipeline: {sel}: {e}");
            (None, Some(e))
        }
    }
}

/// Say when a bundle's chrome policy is not the default.
///
/// A missing window border is the kind of change that gets reported as a bug
/// against the compositor rather than against the bundle that asked for it, so it
/// is announced. The policy itself is not published anywhere — it lives on the
/// bundle, which lives on this world's `Two`, and the drawing code reads it there.
fn announce_chrome(cp: &CompiledPipeline) {
    use compositor_pipeline_bundle_manifest_base::manifest::{Decorations, LetterboxMode};
    let no_border = cp.decorations == Decorations::Off;
    if no_border || cp.letterbox != LetterboxMode::Keep {
        info!(
            "background.pipeline: window chrome — border={}, letterbox={:?}",
            !no_border, cp.letterbox
        );
    }
}

/// Resolve the active background shader for `selection` (a bundle name/path):
/// a runtime-loaded GLES program or Vulkan module for the active renderer,
/// falling back to the built-in `spacev3` (GLES) / native parallax (Vulkan).
/// `optimized` picks the source's cheap variant when it declares one — unlike the
/// stock parallax (whose two variants are both compiled in and chosen per draw),
/// a runtime shader bakes the flag into its SPIR-V, so it is resolved HERE and a
/// change of the flag has to come back through a rebuild.
/// Also returns the effective `@prop` params: the shader's declared defaults,
/// with `params_override` (the per-world edited values) overlaid slot-for-slot.
pub fn build(
    renderer: &mut GlesRenderer,
    selection: Option<&str>,
    params_override: &[(String, f32)],
    optimized: bool,
) -> (Option<GlesPixelProgram>, Option<Arc<VulkanModule>>, [f32; 16], Option<String>) {
    let prefers_dmabuf =
        compositor_model_stats_registry_base::base::compositor_prefers_dmabuf();
    let (loaded, error) = match selection {
        Some(s) => compositor_pipeline_bundle_load_base::load(renderer, prefers_dmabuf, s, optimized),
        None => (None, None),
    };
    // Effective params: the shader's declared defaults, then this world's overrides
    // matched by prop NAME (slot = the prop's index in declaration order).
    let props = loaded
        .as_ref()
        .map(|l| l.properties.clone())
        .unwrap_or_else(compositor_pipeline_bundle_builtin_base::builtin_props);
    let mut params = compositor_pipeline_bundle_property_base::default_params(&props);
    for (name, val) in params_override {
        if let Some(slot) = props.iter().position(|p| &p.name == name) {
            if slot < 16 {
                params[slot] = *val;
            }
        }
    }
    let (loaded_gles, vulkan) = match loaded {
        Some(l) => (l.gles, l.vulkan.map(Arc::new)),
        None => (None, None),
    };
    // GLES: loaded program or built-in. Vulkan: no GLES program (never sampled).
    let program = if prefers_dmabuf {
        None
    } else {
        loaded_gles
            .or_else(|| Some(compile_program(renderer)))
    };
    (program, vulkan, params, error)
}

/// The dispatch-seam pass for a runtime-loaded Vulkan shader: SDR only (no HDR
/// variant — the renderer reuses `sdr`), with the standard engine + params push.
pub fn loaded_pass<'a>(
    m: &'a VulkanModule,
    u: &ParallaxUniforms,
    params: &[f32; 16],
) -> NativeShaderPass {
    NativeShaderPass {
        sdr: ShaderVariant {
            id: m.id,
            spv: Arc::clone(&m.spv),
            vert_spv: m.vert_spv.clone(),
            vert_entry: Arc::clone(&m.vert_entry),
            frag_entry: Arc::clone(&m.frag_entry),
            push: std::sync::Arc::from(
                compositor_pipeline_abi_push_base::base::engine_push(u, params).to_vec(),
            ),
        },
        hdr: None,
        pipeline: None,
    }
}

/// The baked-in `spacev3.frag` GLES program; a baked-in shader must always
/// compile, so a failure aborts rather than falling back.
///
/// Lives here rather than in `pipeline.compile/compile.program` because the
/// source it bakes in is background CONTENT — it sits in `draw.element/shaders`
/// next door. The pipeline crate keeps `compile_source`, which compiles an
/// arbitrary GLES bundle and knows nothing about which shader it is.
pub fn compile_program(renderer: &mut GlesRenderer) -> GlesPixelProgram {
    match compositor_pipeline_compile_program_base::compile_source(
        renderer,
        include_str!("../draw.element/shaders/spacev3.frag"),
    ) {
        Ok(program) => program,
        Err(err) => abort!("Failed to compile GLES shader {err:?}"),
    }
}
