//! `submit_frame`: replay the queued draw ops into one composite pass and
//! submit (SDR or HDR record path; synchronous or native-KMS-IN_FENCE sync).

use ash::vk;
use smithay::backend::renderer::sync::SyncPoint;
use compositor_developer_stats_registry_base::base as stats;

use crate::error::VulkanError;
use crate::frame::DrawOp;
use super::VulkanRenderer;

impl VulkanRenderer {
    /// Replay the queued draw ops into one composite pass and submit. Called by
    /// `VulkanFrame::finish`.
    pub(crate) fn submit_frame(
        &mut self,
        image: vk::Image,
        view: vk::ImageView,
        format: vk::Format,
        extent: (u32, u32),
        clear: [f32; 4],
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
            if let DrawOp::ShaderPass { sdr, hdr } = op {
                let v = if use_hdr { hdr.as_ref().unwrap_or(sdr) } else { sdr };
                self.ensure_shader_pass(v, format)?;
            }
        }

        // Live graphics config (settings "Graphics" tab, via preferences) +
        // current world zoom → the effective, zoom-weighted AA knobs. AA applies
        // to the SDR composite path only (HDR skips it). The pipeline is built
        // lazily on activation and torn down on deactivation.
        let gfx = compositor_developer_environment_graphics_base::base::get();
        let zoom = compositor_developer_stats_registry_base::base::world_zoom() as f32;
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

        self.frame_counter += 1;
        let value = self.frame_counter;
        stats::frame();

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
            let t = compositor_developer_stats_registry_base::base::hdr_tuning();
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
                |_cmd| {},
                |cmd| {
                    hdr.begin_frame(dev, cmd);
                    for op in ops.iter() {
                        match op {
                            DrawOp::Solid { quad } => hdr.draw_solid(dev, cmd, to_push(quad, sdr)),
                            DrawOp::ShaderPass { sdr: s, hdr: h } => {
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
            use compositor_developer_environment_graphics_base::base::AaMethod;
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
            compositor_kernel_vulkan_command_record_base::record::record_composition(
                dev,
                cmd,
                image,
                view,
                extent,
                clear,
                pipelines,
                |cmd| {
                    // Pre-pass: (re)generate the mip chain for each AA mip op.
                    if !mip_jobs.is_empty() {
                        if let Some(aa) = aa {
                            let mg = mipgen.borrow();
                            for (idx, fill) in &mip_jobs {
                                mg.record(dev, cmd, aa, *fill, *idx);
                            }
                        }
                    }
                },
                |cmd| {
                    for (i, (op, set)) in ops.iter().zip(sets.iter()).enumerate() {
                        match op {
                            DrawOp::Solid { quad } => {
                                compositor_kernel_vulkan_element_solid_base::solid::draw(
                                    dev, pipelines, cmd, *quad,
                                );
                            }
                            DrawOp::ShaderPass { sdr, hdr } => {
                                let v = if use_hdr { hdr.as_ref().unwrap_or(sdr) } else { sdr };
                                if let Some(fp) = shader_passes.get(&(v.id, format)) {
                                    fp.draw(dev, cmd, &v.push);
                                }
                            }
                            DrawOp::Textured { quad, tex_w, tex_h, .. } => {
                                let set = set.expect("textured op has a set");
                                if aa_op[i] {
                                    let aa = aa.expect("aa pipeline present for aa op");
                                    aa.draw(
                                        dev,
                                        cmd,
                                        set,
                                        compositor_kernel_vulkan_pipeline_composite_base::composite::AaPush {
                                            dst: quad.dst,
                                            src: quad.src,
                                            color: quad.color,
                                            params: [aa_taps as f32, aa_spread, aa_sharpen, aa_lod_bias],
                                            params2: [
                                                if aa_easu { 1.0 } else { 0.0 },
                                                if aa_rcas { aa_rcas_strength } else { 0.0 },
                                                *tex_w as f32,
                                                *tex_h as f32,
                                            ],
                                        },
                                    );
                                } else {
                                    compositor_kernel_vulkan_element_texture_base::texture::draw(
                                        dev, pipelines, cmd, set, *quad,
                                    );
                                }
                            }
                        }
                    }
                },
            )
            .map_err(|e| VulkanError::Vk(format!("record: {e}")))?;
        }

        if !self.use_native_fence() {
            // Synchronous (the DEFAULT; winit; anything but the infence opt-in):
            // signal the timeline, then device_wait_idle. The returned SyncPoint
            // is already-signaled.
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
            // Post-scene capture (native Vulkan path): copy the now-complete
            // composed scene into the registry entry dmabufs. No-op unless the
            // backend set capture targets for this frame. Ends the `dev` borrow
            // first (it needs `&mut self`'s capture fields).
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
        match compositor_kernel_vulkan_sync_export_base::export::export_sync_file(
            dev,
            self.render_semaphore,
        ) {
            Ok(fd) => {
                stats::fence_kms_infence();
                Ok(SyncPoint::from(crate::sync_fence::SyncFileFence::new(fd)))
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
                Ok(SyncPoint::signaled())
            }
        }
    }
}
