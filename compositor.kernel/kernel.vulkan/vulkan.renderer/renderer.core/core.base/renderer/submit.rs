//! `submit_frame`: replay the queued draw ops into one composite pass and
//! submit (SDR or HDR record path; synchronous or native-KMS-IN_FENCE sync).

use ash::vk;
use smithay::backend::renderer::sync::SyncPoint;
use compositor_model_stats_registry_base::base as stats;

use crate::error::VulkanError;
use crate::frame::DrawOp;
use super::VulkanRenderer;

impl VulkanRenderer {
    /// Replay the queued draw ops into one composite pass and submit. Called by
    /// `VulkanFrame::finish`.
    #[allow(clippy::too_many_arguments)]
    /// Take THIS output's resources for the duration of the pass, and put them
    /// back however it ends.
    ///
    /// Taken rather than borrowed because the body needs `&mut self` throughout
    /// (`ensure_pipelines`, `import_dmabuf`, …) and a live borrow into
    /// `self.outputs` would fight every one of them. Reinserted on the error path
    /// too: a frame that failed still owns real GPU objects, and dropping them on
    /// the floor would leak the whole intermediate target set.
    pub(crate) fn submit_frame(
        &mut self,
        image: vk::Image,
        view: vk::ImageView,
        format: vk::Format,
        extent: (u32, u32),
        clear: [f32; 4],
        clear_rects: Vec<vk::Rect2D>,
        acquire: crate::frame::TargetAcquire,
        ops: Vec<DrawOp>,
    ) -> Result<SyncPoint, VulkanError> {
        let key = std::sync::Arc::clone(&self.output);
        let mut out = self.outputs.remove(&key).unwrap_or_default();
        let r = self.submit_pass(
            &mut out, image, view, format, extent, clear, clear_rects, acquire, ops,
        );
        self.outputs.insert(key, out);
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_pass(
        &mut self,
        out: &mut super::OutputResources,
        image: vk::Image,
        view: vk::ImageView,
        format: vk::Format,
        extent: (u32, u32),
        clear: [f32; 4],
        clear_rects: Vec<vk::Rect2D>,
        acquire: crate::frame::TargetAcquire,
        ops: Vec<DrawOp>,
    ) -> Result<SyncPoint, VulkanError> {
        self.ensure_pipelines(format)?;
        if self.use_hdr() {
            self.ensure_hdr_pipeline(format)?;
        }
        let use_hdr = self.use_hdr();
        // Build any fullscreen-shader-pass pipelines this frame references (a
        // mutable borrow, done before the immutable borrows used to record).
        for op in &ops {
            if let DrawOp::ShaderPass { sdr, hdr, .. } = op {
                let v = if use_hdr { hdr.as_ref().unwrap_or(sdr) } else { sdr };
                self.ensure_shader_pass(v, format)?;
            }
        }

        // The native-fence path reuses one command buffer + descriptor pool and
        // does NOT device_wait_idle, so before re-recording we must ensure the
        // previous frame's GPU work finished. Pace on the VkFence (created
        // signaled, so the first frame passes through). The synchronous path
        // device_wait_idles after submit, so it needs no pre-wait.
        if self.use_native_fence() {
            unsafe {
                self.dev
                    .device
                    .wait_for_fences(&[self.frame_fence], true, u64::MAX)?;
                self.dev.device.reset_fences(&[self.frame_fence])?;
            }
        }
        // The previous frame is provably complete here (fence wait above on
        // the native path; the sync path device_wait_idles inside submit), so
        // the render targets it retired can be destroyed — and its texture
        // pins dropped. The frame being submitted keeps its own pins (pushed
        // during record) until the next drain point.
        self.drain_retired();
        // The warp grid recorded into the frame that just completed. Read HERE
        // because completion is already proven at the drain above, so this is a
        // memcpy and never a wait.
        if let Some(g) =
            compositor_pipeline_execute_warpmap_base::base::WarpMap::publish(&mut out.warpmap, &self.dev)
        {
            out.warp_grid = Some(g);
        }
        // The previous frame is provably complete at this point, so its `content`
        // is safe for another device to sample — the same completion-before-publish
        // rule the worker's own output follows. Publishing here rather than at the
        // end of recording is what makes it true.
        //
        // Only while there is an offscreen path to publish FROM: with no bundle
        // there is no `ContentPath`, so this would be two writes per frame storing
        // `None` over `None`. The latch keeps the WITHDRAWAL — the frame the path
        // disappears still publishes `None`, so a stale `content` cannot outlive
        // the bundle that produced it.
        let share_now = out.offscreen.is_some() || out.shared_content;
        out.shared_content = out.offscreen.is_some();
        if share_now {
            out.content_shared = out.offscreen.as_ref().and_then(|o| o.content_share());
        }
        // The window layer rides the same proof and the same drain point. It is
        // `None` for every bundle that does not sample `windows`, because the
        // layer itself is consumption-gated — so this costs a refcount, not an
        // image, on the overwhelmingly common path.
        if share_now {
            out.windows_shared = out.offscreen.as_ref().and_then(|o| o.windows_share());
        }
        // Same proven-complete precondition as the drain above, so cached imports
        // whose dmabuf has died are reclaimed here.
        self.reap_targets();
        self.in_flight_textures = std::mem::take(&mut self.pinned_textures);

        // Compose only what smithay says changed. Requires a target whose
        // contents we can take over (see `TargetAcquire`); the HDR branch below
        // keeps the full-clear shape, so it opts out too.
        let acquire_layout = acquire.old_layout().filter(|_| !self.use_hdr());
        let damaged = acquire_layout.is_some();

        // Textures served from the import cache since the last submit (already
        // deduped on push). Drained here, so the barrier is paid once for
        // everything that accumulated over any frames that never submitted.
        let acquires = std::mem::take(&mut self.pending_acquires);

        // Live graphics config (settings "Graphics" tab, via preferences) +
        // current world zoom → the effective, zoom-weighted AA knobs. AA applies
        // to the SDR composite path only (HDR skips it). The pipeline is built
        // lazily on activation and torn down on deactivation. Placed AFTER the
        // drain point above: the teardown destroys objects the previous frame
        // may have had in flight, and needs no drain of its own here.
        let gfx = compositor_model_environment_graphics_base::base::get();
        let zoom = compositor_model_stats_registry_base::base::world_zoom() as f32;
        let eff = gfx.effective(zoom);
        let aa_active = eff.active && !use_hdr;
        if aa_active {
            self.ensure_aa_pipeline(format)?;
        } else if self.aa_was_active {
            // Deactivation edge: reclaim the AA pipeline(s) and per-surface mip
            // images so a disabled AA config costs no resident GPU memory.
            self.teardown_aa();
        }
        self.aa_was_active = aa_active;

        self.frame_counter += 1;
        let value = self.frame_counter;
        stats::frame();

        // What this frame's bundle asks the engine for — the ONE place that is
        // decided, so the composite below reads flags rather than re-deriving
        // predicates. Empty with no bundle loaded. See `worldset::Demand`.
        use compositor_pipeline_execute_offscreen_base::offscreen;
        let demand = super::worldset::Demand::resolve(&ops, self.facts);
        let skip_windows = demand.skip_windows();
        let offscreen_on = demand.offscreen(use_hdr);
        // Stage 4: the worker's decorated band, imported here so the swapchain can
        // sample it. Through the ordinary cache, so an unchanged band costs a
        // lookup. Needs `&mut self`, which is why it is not part of `Demand`.
        let band_tex = self.after_band.take()
            .and_then(|b| {
                use smithay::backend::renderer::ImportDma;
                self.import_dmabuf(&b, None).ok()
            });
        if offscreen_on {
            let rebuild = out.offscreen.as_ref().map(|o| o.format() != format).unwrap_or(true);
            if rebuild {
                if let Some(o) = out.offscreen.take() {
                    o.destroy(&self.dev);
                }
                out.offscreen =
                    Some(offscreen::ContentPath::new(&self.dev, self.pipeline_cache, format)?);
            }
        }

        let collected = demand.gather(&ops, extent);
        // Handed out through `take_world_set`, so the world that was drawn owns it
        // rather than every reader sharing one slot. `None` when the bundle asked
        // for nothing, which withdraws whatever the world was holding.
        out.world_set = match demand.requires.world_set() {
            true => Some(std::sync::Arc::new(collected.to_world_set())),
            false => None,
        };
        let bound = collected.bind(&demand.claim);


        // A pipeline op in THIS frame means the graph runs inline: build its
        // intermediates and bind the set to the compositor's own executor.
        if ops.iter().any(|o| matches!(o, DrawOp::Pipeline(_))) {
            let mem = unsafe {
                self.phd
                    .instance()
                    .handle()
                    .get_physical_device_memory_properties(self.phd.handle())
            };
            let cache = self.pipeline_cache;
            for op in &ops {
                if let DrawOp::Pipeline(p) = op {
                    out.graph.prepare(&self.dev, &mem, cache, p, extent, format)?;
                    if compositor_pipeline_execute_warpmap_base::base::WarpMap::sync(
                        &self.dev, &mem, cache, p.warp_map.as_ref(), &mut out.warpmap, "inline",
                    )? {
                        // The producer changed or went: a grid outliving its bundle
                        // displaces the pointer by an effect no longer on screen.
                        out.warp_grid = None;
                    }
                }
            }
            out.graph.set_world_entries(&self.dev, &bound.rects, &bound.srcs, &bound.meta);
            // Empty unless the bundle declared `window_times`; the call is a no-op
            // then, so this costs nothing to leave unconditional.
            out.graph.set_world_times(&self.dev, &bound.life, &bound.state, &bound.drag);
            if demand.requires.pointer() {
                out.graph.set_pointer(&self.dev);
            }
            out.graph.set_window_textures(&self.dev, &bound.views);
        } else if !self.facts.active() {
            // No bundle loaded at all: release this output's intermediates. The
            // branch above only ever frees as a side effect of building the next
            // pipeline, so switching from a multipass bundle back to the built-in
            // parallax left its targets — ~95 MiB per output here — allocated for
            // the rest of the session.
            //
            // Gated on FACTS, not on "this frame carried no pipeline op". A loaded
            // bundle can miss a frame; tearing down on that would free and rebuild
            // every target the frame after, which is the thrash the per-output
            // keying exists to avoid. `Facts::active()` follows the bundle, so it
            // only goes false when one is genuinely unloaded.
            if out.graph.release(&self.dev) {
                info!("pipeline: no bundle — released {} graph intermediates", self.output);
            }
            if let Some(w) = out.warpmap.take() {
                w.destroy(&self.dev);
            }
            // And the offscreen path, for the same reason: `content` is allocated
            // by `record` and `record` only runs while a pipeline does, so an
            // unloaded bundle left a full output-sized image (~30 MiB) behind. The
            // whole `ContentPath` goes rather than just the image — it is rebuilt
            // on demand above whenever `offscreen_on`, and the shares must not
            // outlive the images they name.
            if let Some(o) = out.offscreen.take() {
                o.destroy(&self.dev);
                out.content_shared = None;
                out.windows_shared = None;
                info!("pipeline: no bundle — released {} offscreen content", self.output);
            }
            // A grid outliving its producer displaces the pointer by an effect
            // that is no longer on screen — same reason as the `sync` arm above.
            out.warp_grid = None;
        }

        let this = &*self;
        let dev = &self.dev;
        let pipelines = self
            .pipelines
            .get(&format)
            .expect("pipelines ensured above");
        let shader_passes = &self.shader_passes;
        let pool = self.descriptor_pool;
        let cmd = self.cmd;

        if self.use_hdr() {
            // HDR path (M5 1a): composite via the WGSL HDR pipeline.
            let hdr = self
                .hdr_pipelines
                .get(&format)
                .expect("hdr pipeline ensured above");
            let t = compositor_model_stats_registry_base::base::hdr_tuning();
            hdr.update_tuning(&crate::hdr_composite::HdrTuningUbo {
                enabled: t.enabled,
                sdr_white_nits: t.sdr_white_nits,
                max_nits: t.max_nits,
                brightness: t.brightness,
                contrast: t.contrast,
                saturation: t.saturation,
                gamut: t.gamut,
                tone_map: t.tone_map,
                transfer: t.transfer,
                gamma: t.gamma,
                exposure: t.exposure,
                _pad: 0.0,
            });
            let to_push = |q: &compositor_kernel_vulkan_pipeline_composite_base::composite::PushQuad,
                           surf: [f32; 4]| {
                crate::hdr_composite::HdrPush {
                    dst: q.dst,
                    src: q.src,
                    color: q.color,
                    surf,
                }
            };
            let sdr = [0.0_f32; 4];
            compositor_kernel_vulkan_command_record_base::record::record_composition(
                dev, cmd, image, view, extent, clear, pipelines,
                None,
                |cmd| this.record_pending_acquires(cmd, &acquires),
                |cmd| {
                    hdr.begin_frame(dev, cmd);
                    for op in ops.iter() {
                        match op {
                            DrawOp::Solid { quad, .. } => hdr.draw_solid(dev, cmd, to_push(quad, sdr)),
                            DrawOp::ShaderPass { sdr: s, hdr: h, .. } => {
                                let v = if use_hdr { h.as_ref().unwrap_or(s) } else { s };
                                if let Some(fp) = shader_passes.get(&(v.id, format)) {
                                    fp.draw(dev, cmd, &v.push);
                                }
                            }
                            DrawOp::Textured { quad, view: v, surf, .. } => {
                                match hdr.texture_set(dev, *v) {
                                    Ok(set) => hdr.draw_textured(dev, cmd, set, to_push(quad, *surf)),
                                    Err(e) => warn!("hdr texture set: {e}"),
                                }
                            }
                            // Multipass background not yet wired for HDR output;
                            // the SDR path drives it. Skip rather than mis-draw.
                            DrawOp::Pipeline(_) => {}
                        }
                    }
                },
            )
            .map_err(|e| VulkanError::Vk(format!("hdr record: {e}")))?;
        } else {
            // world anti-aliasing on: textured (window + iced) draws route through the AA
            // pipeline (its own separate-binding sets from its own pool);
            // solids + the parallax shader-pass are untouched. Off: the plain
            // combined-sampler composite path below.
            let aa = if aa_active {
                self.aa_pipelines.get(&format)
            } else {
                None
            };
            let aa_taps = eff.taps;
            let aa_spread = eff.spread;
            let aa_sharpen = eff.sharpen;
            let aa_lod_bias = eff.lod_bias;
            let aa_aniso = eff.aniso;
            // FSR toggles (independent of the AA method). When either is on the
            // shader takes over base sampling, so the classic mip methods (and
            // their per-surface mip pre-pass) are bypassed for those draws.
            let aa_easu = eff.easu;
            let aa_rcas = eff.rcas;
            let aa_rcas_strength = eff.rcas_sharpen;
            let fsr = aa_easu || aa_rcas;
            // Which pre-built sampler the composite draws bind for this method.
            use compositor_kernel_vulkan_pipeline_composite_base::composite::SamplerSel;
            use compositor_model_environment_graphics_base::base::AaMethod;
            let comp_sel = if fsr {
                // EASU/RCAS fetch integer texels (textureLoad) and only bilinear-
                // sample for alpha; the mip samplers don't apply.
                SamplerSel::Bilinear
            } else {
                match eff.method {
                    AaMethod::Trilinear => SamplerSel::Trilinear,
                    AaMethod::Anisotropic => SamplerSel::Aniso,
                    _ => SamplerSel::Bilinear,
                }
            };
            // AA is decided PER OP: only minified world content (windows +
            // iced-world) is eligible — screen-space iced and the 1:1 bevy
            // background stay on the plain composite path. Aniso/trilinear also
            // need a per-surface mip chain (render-to-mip0 + blit-down) before
            // the composite pass — skipped when an FSR filter overrides sampling.
            let method_mips = aa_active && eff.method.needs_mips() && !fsr;
            let mem_props = if method_mips {
                Some(unsafe {
                    self.phd
                        .instance()
                        .handle()
                        .get_physical_device_memory_properties(self.phd.handle())
                })
            } else {
                None
            };
            if let Some(aa) = aa {
                aa.begin_frame(dev);
            }
            unsafe {
                dev.device
                    .reset_descriptor_pool(pool, vk::DescriptorPoolResetFlags::empty())?;
            }

            // Per textured op: its descriptor set, whether it draws via the AA
            // pipeline (`aa_op`), and (for mip methods) its mip pre-pass job.
            let mut sets: Vec<Option<vk::DescriptorSet>> = Vec::with_capacity(ops.len());
            let mut aa_op: Vec<bool> = Vec::with_capacity(ops.len());
            let mut mip_jobs: Vec<(usize, vk::DescriptorSet)> = Vec::new();
            {
                let mut mg = self.mipgen.borrow_mut();
                if method_mips {
                    mg.begin_frame();
                }
                for op in &ops {
                    match op {
                        // Owned by the pipeline (§8d): no descriptor set, and no AA
                        // mip chain — those come from a per-frame capped pool, and
                        // generating one for a surface the engine never draws would
                        // starve the surfaces it does.
                        // Only skip the set + AA mip when NOTHING will draw this
                        // window: suppressed inline AND not wanted for the layer.
                        DrawOp::Textured { meta, .. }
                            if !demand.keep_windows && demand.claim.owns(meta) =>
                        {
                            sets.push(None);
                            aa_op.push(false);
                        }
                        DrawOp::Textured { view, tex_w, tex_h, meta, .. } => {
                            // Eligible world op? For mip methods, also try to
                            // claim a mip image (None = over the per-frame cap →
                            // fall back to the plain path for this surface).
                            let mut mip_idx = None;
                            let use_aa = if aa_active && meta.is_world() {
                                if method_mips {
                                    let mp = mem_props.as_ref().unwrap();
                                    mip_idx = mg
                                        .acquire(dev, mp, format, *tex_w, *tex_h)
                                        .map_err(|e| VulkanError::Vk(format!("mip acquire: {e}")))?;
                                    mip_idx.is_some()
                                } else {
                                    true
                                }
                            } else {
                                false
                            };
                            if use_aa {
                                let aa = aa.expect("aa pipeline ensured when aa_active");
                                let set = match mip_idx {
                                    Some(idx) => {
                                        let fill = aa
                                            .texture_set(dev, *view, SamplerSel::Bilinear, 1.0)
                                            .map_err(|e| VulkanError::Vk(format!("mip fill set: {e}")))?;
                                        mip_jobs.push((idx, fill));
                                        aa.texture_set(dev, mg.view_of(idx), comp_sel, aa_aniso)
                                            .map_err(|e| VulkanError::Vk(format!("mip comp set: {e}")))?
                                    }
                                    None => aa
                                        .texture_set(dev, *view, comp_sel, aa_aniso)
                                        .map_err(|e| VulkanError::Vk(format!("aa set: {e}")))?,
                                };
                                sets.push(Some(set));
                                aa_op.push(true);
                            } else {
                                let layouts = [pipelines.descriptor_layout];
                                let info = vk::DescriptorSetAllocateInfo::default()
                                    .descriptor_pool(pool)
                                    .set_layouts(&layouts);
                                let set = unsafe { dev.device.allocate_descriptor_sets(&info)? }[0];
                                compositor_kernel_vulkan_element_texture_base::texture::bind_texture(
                                    dev, pipelines, set, *view,
                                );
                                sets.push(Some(set));
                                aa_op.push(false);
                            }
                        }
                        _ => {
                            sets.push(None);
                            aa_op.push(false);
                        }
                    }
                }
            }

            let mipgen = &self.mipgen;
            let damage_pass = acquire_layout.map(|old_layout| {
                compositor_kernel_vulkan_command_record_base::record::DamagePass {
                    clear_rects: &clear_rects,
                    old_layout,
                }
            });
            // The offscreen/multipass path forces a full-frame redraw: after-content
            // passes sample NEIGHBOURING pixels of `content`, so a damage-scissored
            // partial update would post-process stale edges.
            //
            // An owned band does NOT, though it used to. The pipeline may place
            // window pixels anywhere, but the band it draws them into is a
            // full-output element whose own damage is the whole output on every
            // frame it changes — so the spill is already covered, and collapsing
            // damage on top of that cost a full composite on every frame that
            // touched anything. What an owned band does need is for the drawables
            // it takes over to stop claiming opacity, so the band repaints under
            // them; see `bridge.window::set::owned_by_bundle`.
            let damaged = damaged && !offscreen_on;
            // Detach the content path so the closures below can hold `&*self`
            // (the graph + the pending-acquire recorder) while it is driven
            // mutably. Restored right after the record call.
            let mut off = out.offscreen.take();
            // Detached for the same reason as `offscreen`: the closures below hold
            // `&*self` while this is driven mutably. Restored right after.
            let mut warpmap = out.warpmap.take();
            let this = &*self;
            let graph = &out.graph;
            let pre = |cmd: vk::CommandBuffer| {
                // Cache-served imports first: the mip pre-pass below samples
                // them, so their writes must be visible before it runs.
                this.record_pending_acquires(cmd, &acquires);
                // Pre-pass: (re)generate the mip chain for each AA mip op.
                if !mip_jobs.is_empty() {
                    if let Some(aa) = aa {
                        let mg = mipgen.borrow();
                        for (idx, fill) in &mip_jobs {
                            mg.record(dev, cmd, aa, *fill, *idx);
                        }
                    }
                }
                // Multipass background: render each pipeline's intermediate passes
                // into their offscreen targets (outside the composite pass).
                for op in ops.iter() {
                    if let DrawOp::Pipeline(p) = op {
                        graph.record_intermediates(dev, cmd, p, value);
                        // The warp grid renders into its OWN tiny target, outside
                        // the composite pass, and copies itself to host memory in
                        // the same command buffer. Nothing waits — the bytes are
                        // read at the next drain, where this frame is complete.
                        if let (Some(w), Some(g)) = (warpmap.as_mut(), p.warp_map.as_ref()) {
                            w.record(dev, cmd, &g.push);
                        }
                    }
                }
            };
            // World/screen split for after-content layering. Everything up to the
            // last world-ish op (the background pipeline/shader + world windows) is
            // the WORLD band — composited into `content` and post-processed by the
            // after-content passes. The trailing ops (layer-shell top, iced-screen
            // UI, pointer) are the SCREEN band, drawn ON TOP of the post-processed
            // result so the vignette/glass never darkens the UI or cursor. Only
            // split when compositing offscreen for an after-content pipeline.
            let split_at = if demand.has_after {
                ops.iter()
                    .rposition(|o| {
                        matches!(o, DrawOp::Pipeline(_))
                            || matches!(o, DrawOp::ShaderPass { .. })
                            || matches!(o, DrawOp::Textured { meta, .. } if meta.is_world())
                    })
                    .map(|i| i + 1)
                    .unwrap_or(ops.len())
            } else {
                ops.len()
            };
            let composer = super::compose::Composer {
                dev,
                pipelines,
                shader_passes,
                graph,
                aa,
                ops: &ops,
                sets: &sets,
                aa_op: &aa_op,
                clear_rects: &clear_rects,
                demand: &demand,
                aa_params: super::compose::Aa {
                    taps: aa_taps,
                    spread: aa_spread,
                    sharpen: aa_sharpen,
                    lod_bias: aa_lod_bias,
                    easu: aa_easu,
                    rcas: aa_rcas,
                    rcas_strength: aa_rcas_strength,
                },
                tick: value,
                use_hdr,
                format,
                damaged,
                skip_windows,
                split_at,
            };
            let compose = |cmd: vk::CommandBuffer| composer.compose(cmd);
            let compose_windows = |cmd: vk::CommandBuffer| composer.compose_windows(cmd);
            let compose_screen = |cmd: vk::CommandBuffer| composer.compose_screen(cmd);
            if offscreen_on {
                // Owned memory props (Copy) — borrow of `phd` ends before the
                // `&mut out.offscreen` borrow below (disjoint fields).
                let off_mem = unsafe {
                    self.phd
                        .instance()
                        .handle()
                        .get_physical_device_memory_properties(self.phd.handle())
                };
                // After-content pipeline (if any) samples `content` in the post
                // stage; otherwise the offscreen path does its passthrough.
                let after_pipe = ops.iter().find_map(|o| match o {
                    DrawOp::Pipeline(p) if p.has_after() => Some(p.clone()),
                    _ => None,
                });
                let post_pre = |cmd: vk::CommandBuffer,
                                cv: vk::ImageView,
                                hv: vk::ImageView,
                                wv: vk::ImageView| {
                    if let Some(p) = &after_pipe {
                        graph.record_after_intermediates(dev, cmd, p, cv, hv, wv, value);
                    }
                };
                let post_draw = |cmd: vk::CommandBuffer,
                                 cv: vk::ImageView,
                                 hv: vk::ImageView,
                                 wv: vk::ImageView|
                 -> bool {
                    match &after_pipe {
                        Some(p) => {
                            graph.draw_after_output(dev, cmd, p, cv, hv, wv, value);
                            true
                        }
                        None => false,
                    }
                };
                let r = off
                    .as_mut()
                    .expect("offscreen ensured above")
                    .record(
                        dev, &off_mem, cmd, image, view, extent, format, clear, demand.keep_history,
                        demand.keep_windows, band_tex.as_ref().map(|t| t.view()), pre, compose,
                        compose_windows, compose_screen, post_pre, post_draw,
                    )
                    .map_err(|e| VulkanError::Vk(format!("offscreen record: {e}")));
                out.offscreen = off;
                out.warpmap = warpmap.take();
                r?;
            } else {
                let r = compositor_kernel_vulkan_command_record_base::record::record_composition(
                    dev, cmd, image, view, extent, clear, pipelines, damage_pass, pre, compose,
                )
                .map_err(|e| VulkanError::Vk(format!("record: {e}")));
                out.offscreen = off;
                out.warpmap = warpmap.take();
                r?;
            }
        }

        if !self.use_native_fence() {
            // Synchronous (winit, which has no DRM fd; or the explicit
            // `renderer_sync = "sync"` opt-out; or a failed self-test): signal
            // the timeline, then device_wait_idle. The returned SyncPoint is
            // already-signaled.
            compositor_kernel_vulkan_device_queue_base::queue::submit_with_timeline(
                dev,
                &self.queue,
                cmd,
                self.timeline,
                value,
            )
            .map_err(VulkanError::Vk)?;
            unsafe {
                dev.device.device_wait_idle()?;
            }
            stats::fence_synchronous();
            // Post-scene capture: copy the now-complete composed scene into
            // the registry entry dmabufs. No-op unless the backend set capture
            // targets for this frame. Ends the `dev` borrow first (it needs
            // `&mut self`'s capture fields). No wait point — the drain above
            // already completed the composite.
            let _ = dev;
            let targets = std::mem::take(&mut self.capture_targets);
            compositor_kernel_vulkan_capture_blit_base::blit::blit_into_targets(
                &self.dev,
                self.command_pool,
                self.queue.queue,
                &mut self.capture_cache,
                image,
                extent,
                &targets,
                None,
            );
            return Ok(SyncPoint::signaled());
        }

        // Native KMS path: submit signalling the binary render semaphore and the
        // VkFence (CPU pacing), WITHOUT device_wait_idle, then export the
        // semaphore's pending signal directly as a `sync_file` fd for the
        // atomic-commit IN_FENCE.
        let cmd_info = vk::CommandBufferSubmitInfo::default().command_buffer(cmd);
        let timeline_sig = vk::SemaphoreSubmitInfo::default()
            .semaphore(self.timeline)
            .value(value)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS);
        let render_sig = vk::SemaphoreSubmitInfo::default()
            .semaphore(self.render_semaphore)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS);
        let signals = [timeline_sig, render_sig];
        let submit = vk::SubmitInfo2::default()
            .command_buffer_infos(std::slice::from_ref(&cmd_info))
            .signal_semaphore_infos(&signals);
        unsafe {
            dev.device
                .queue_submit2(self.queue.queue, &[submit], self.frame_fence)
                .map_err(|e| VulkanError::Vk(format!("queue_submit2: {e}")))?;
        }
        let sync = match compositor_kernel_vulkan_sync_export_base::export::export_sync_file(
            dev,
            self.render_semaphore,
        ) {
            Ok(fd) if self.fence_fallback_optin && !self.fence_validated => {
                // First-frame self-test, ONLY under infence_fallback_sync: a
                // dud exported fence would park the queued atomic commit
                // forever (the historical infence freeze). Costs one
                // composite-length CPU wait, once. Plain `infence` runs no
                // validation at all — raw behavior, so broken fence stacks
                // can be observed as they actually fail.
                use std::os::unix::io::AsFd;
                let drm_fd = self.drm_fd.as_ref().expect("native fence path has drm_fd");
                if compositor_kernel_drm_syncobj_device_base::device::sync_file_signals_within(
                    drm_fd,
                    fd.as_fd(),
                    std::time::Duration::from_secs(1),
                ) {
                    self.fence_validated = true;
                    info!("native KMS IN_FENCE self-test passed: first exported fence signaled");
                    stats::fence_kms_infence();
                    SyncPoint::from(crate::sync_fence::SyncFileFence::new(fd))
                } else {
                    warn!(
                        "native KMS IN_FENCE self-test FAILED: exported sync_file did not \
                         signal within 1s; falling back to synchronous mode permanently \
                         (renderer_sync=infence_fallback_sync)"
                    );
                    self.native_fence_optin = false;
                    stats::set_sync_mode("synchronous (device_wait_idle; IN_FENCE self-test failed)");
                    unsafe { dev.device.device_wait_idle()? };
                    stats::fence_fallback();
                    SyncPoint::signaled()
                }
            }
            Ok(fd) => {
                stats::fence_kms_infence();
                SyncPoint::from(crate::sync_fence::SyncFileFence::new(fd))
            }
            Err(e) => {
                if self
                    .last_fence_warn
                    .is_none_or(|t| t.elapsed().as_secs() >= 60)
                {
                    warn!("native KMS fence export failed ({e}); draining device (throttled: once/min)");
                    self.last_fence_warn = Some(std::time::Instant::now());
                }
                unsafe { dev.device.device_wait_idle()? };
                stats::fence_fallback();
                SyncPoint::signaled()
            }
        };
        // Post-scene capture on the native path: a second submission on the
        // same queue, GPU-ordered after the composite by waiting its timeline
        // point; the CPU wait inside is scoped to the blit's fence, paid only
        // on frames that capture. Scanout's IN_FENCE above never waits for
        // this. Ends the `dev` borrow first (capture fields need `&mut self`).
        let _ = dev;
        let targets = std::mem::take(&mut self.capture_targets);
        compositor_kernel_vulkan_capture_blit_base::blit::blit_into_targets(
            &self.dev,
            self.command_pool,
            self.queue.queue,
            &mut self.capture_cache,
            image,
            extent,
            &targets,
            Some((self.timeline, value)),
        );
        Ok(sync)
    }
}
