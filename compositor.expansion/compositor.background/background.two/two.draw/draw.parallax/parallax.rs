use compositor_background_two_draw_motion::Motion;
use compositor_pipeline_compile_spirv_base::VulkanModule;
use compositor_background_two_worker_base::base::Worker;
use compositor_background_two_worker_signal::signal::DrawRequest;
use smithay::backend::allocator::dmabuf::Dmabuf;
use compositor_pipeline_abi_worldset_base::base::Own;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use smithay::backend::renderer::RendererSuper;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesRenderer};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct ParallaxBackground {
    id: Id,
    commit: CommitCounter,
    /// `None` when compositing from dmabufs (Vulkan; a native shader runs).
    program: Option<GlesPixelProgram>,
    /// A runtime-loaded Vulkan shader; `None` runs the built-in pass. `Arc` keeps
    /// the element cheap to `Clone` (it is cloned per frame plan).
    vulkan: Option<Arc<VulkanModule>>,
    /// A multipass (`pipeline.json`) bundle; when set it runs INSTEAD of `vulkan`
    /// (Vulkan only — GLES ignores it and uses the single-pass fallback).
    /// The loaded multipass bundle, if any. `pub` because it is where this world's
    /// window-chrome policy and pointer warp LIVE — readers reach it through the
    /// world's `Two` slot rather than through a process-global copy.
    pub pipeline:
        Option<Arc<compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline>>,
    start_time: Instant,
    pub lock_time: Option<Instant>,
    pub output_size: (f32, f32),
    /// Render-rect top-left (physical px); a pane's origin per-pane, else `(0,0)`.
    pub offset: (i32, i32),
    pub pan: (f32, f32), // state passed from your main loop
    pub zoom: f32,
    /// Shader-authored `@prop` values (16 float slots), fed to the shader each
    /// draw as `u_param0`..`u_param3` (GLES) / the push `params` block (Vulkan).
    pub params: [f32; 16],
    /// The selected shader's compile error for the active renderer, if it failed
    /// (the built-in is rendering instead). Surfaced by the settings panel.
    pub shader_error: Option<String>,
    /// Per-world pan inversion (mirrored from the world's `Two` slot): flip the
    /// camera pan on that axis before feeding the shader. Applied in `draw()`, so
    /// every render path (main, capture, lock, picker) honours it.
    pub invert_pan_x: bool,
    pub invert_pan_y: bool,
    /// Per-world sRGB output flag (mirrored from the world's `Two` slot): when set,
    /// the shader gamma-encodes its final colour so the non-sRGB scanout buffer
    /// shows the brighter, preview-matching look. Carried to the shader in the push.
    pub srgb: bool,
    /// Per-world "Optimized" flag (mirrored from the world's `Two` slot): render
    /// the built-in parallax with its cheap variant. Unlike `srgb` this is not a
    /// push lane — it selects a different SPIR-V — so it only has meaning when the
    /// built-in pass is running (`vulkan` is `None`); a runtime-loaded shader has
    /// no optimized twin and ignores it.
    pub optimized: bool,
    motion: Motion,
    /// The off-thread background worker, when `background_offthread` is on and
    /// the renderer is Vulkan. `Arc` so the per-pane, per-frame clones of this
    /// element stay cheap — every clone drives the SAME worker.
    pub worker: Option<Arc<Worker>>,
    /// Whether the off-thread path was REQUESTED. Distinct from `worker.is_some()`
    /// so a worker that failed to start draws no background instead of silently
    /// falling back to the inline shader — silent fallback would hide exactly the
    /// condition the worker exists to remove.
    pub offthread: bool,
    /// The world this instance's background BELONGS TO — its provenance, which is
    /// deliberately not "the active world". An overlay draws the spawn target's
    /// background (the lock screen, and the `scene.frame` fallback for any world
    /// with no parallax of its own), so resolving the world at bind time would
    /// file that pane under `LOCK_WORLD` and hand it a different buffer set than
    /// the one the same background uses unlocked.
    ///
    /// Stamped where the instance is CONSTRUCTED, which is the only place that
    /// knows: `TwoSystem::buffer` (from `BufferCx::world`) and the picker's own
    /// direct construction. `None` means unstamped — see `worker_frame`.
    pub world: Option<uuid::Uuid>,
    /// This world's drawables, as the renderer collected them last frame.
    ///
    /// Stamped where the element is taken out of the world (which is the only
    /// place with access to its `Storage`) so the two consumers that have only
    /// `&self` — the offload verdict and the worker ping — read the world's own
    /// set instead of a global anything could have written.
    pub world_set: Option<std::sync::Arc<compositor_pipeline_abi_worldset_base::base::WorldSet>>,
    /// The composited band and window layer this world's device exported, for a
    /// stage-4 bundle whose after pass runs on the worker. Stamped beside
    /// `world_set` and carried on the same request.
    pub content_share: Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    pub windows_share: Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
    /// Which pane this clone draws. Panes must not share a buffer: each has its
    /// own camera and physical size, so across monitors a shared buffer would be
    /// the wrong resolution and the wrong view for all but one of them — and
    /// across worlds it would be another world's accumulated pixels.
    ///
    /// `None` until `bind_pane`/`bind_overlay` supplies it.
    pub pane: Option<compositor_background_two_worker_key::key::PaneKey>,
    /// Refresh of the monitor this pane is drawn on, and that output's monotonic
    /// scene-build counter. Both are per-monitor, which is why they ride along
    /// with the pane rather than being resolved worker-side.
    pub refresh: std::time::Duration,
    pub serial: u64,
    /// How many viewport regions this output has, carried so the worker can
    /// retire panes a collapse left behind without waiting on a timeout.
    pub regions: usize,
}

impl ParallaxBackground {
    /// Build the element. `selection` names a user shader bundle (folder name or
    /// absolute path); if it compiles for the active renderer it replaces the
    /// built-in parallax, else the built-in runs.
    pub fn new(
        renderer: &mut GlesRenderer,
        output_size: (f32, f32),
        selection: Option<&str>,
        params_override: &[(String, f32)],
        optimized: bool,
    ) -> Self {
        let (program, vulkan, mut params, single_error) =
            compositor_background_two_draw_select::build(renderer, selection, params_override, optimized);
        // Multipass bundles (`pipeline.json`) take precedence on Vulkan.
        let (pipeline, multi_error) =
            compositor_background_two_draw_select::load_multipass(selection);
        // A `passes/`-only bundle has no single-pass source, so `build` above found
        // no props and handed back sixteen zeros. Re-resolve against the bundle's
        // OWN union — the same list the settings panel edits, in the same order —
        // or every multipass shader runs with every variable at 0.0 rather than at
        // the defaults its author declared.
        if let Some(cp) = &pipeline {
            params = compositor_pipeline_bundle_property_base::default_params(&cp.properties);
            for (name, value) in params_override {
                if let Some(slot) = cp.properties.iter().position(|p| &p.name == name) {
                    if slot < 16 {
                        params[slot] = *value;
                    }
                }
            }
        }
        // The multipass reason wins when there is one. A `passes/`-only bundle has
        // no single-pass source for `build` to fail on, so `single_error` is `None`
        // for exactly the bundles whose failures used to go unreported — the user
        // got the stock parallax behind a settings panel with nothing wrong in it.
        let shader_error = multi_error.or(single_error);
        Self {
            output_size,
            offset: (0, 0),
            id: Id::new(),
            commit: CommitCounter::default(),
            program,
            vulkan,
            pipeline,
            lock_time: None,
            start_time: Instant::now(),
            pan: (0.0, 0.0), zoom: 1.0, params, shader_error,
            invert_pan_x: false, invert_pan_y: false, srgb: false, optimized,
            motion: Motion::new(),
            worker: None, offthread: false, world: None, world_set: None,
            content_share: None, windows_share: None, pane: None, regions: 1,
            // 30Hz until `bind_pane` supplies the monitor's real refresh. Slow on
            // purpose: the worker turns this into a minimum interval, so guessing
            // high would let the pane render at twice its cap for the first frames.
            refresh: std::time::Duration::from_micros(33_333), serial: 0,
        }
    }

    /// Whether a runtime-loaded shader is driving this instance rather than the
    /// stock parallax. The distinction matters for the Optimized toggle: the stock
    /// parallax carries both variants compiled in and re-picks per draw, so it can
    /// be flipped live, whereas a loaded shader baked the flag into its SPIR-V at
    /// `new()` and has to be rebuilt.
    pub fn uses_loaded_shader(&self) -> bool {
        self.vulkan.is_some()
    }

    /// Whether the off-thread worker can reproduce what this element would draw.
    /// The decision, and the one dynamic refusal in it, live in `draw.offload`.
    pub fn worker_can_render(&self) -> bool {
        compositor_background_two_draw_offload::offload::worker_can_render(
            self.pipeline.as_deref(),
            self.world_set.as_deref(),
        )
    }

    /// Stage 4: this bundle's AFTER band runs on the worker, so the compositor
    /// keeps drawing the head band inline and presents the worker's decorated
    /// result instead of running the after pass itself.
    ///
    /// Distinct from `worker_can_render`, which asks whether the worker replaces
    /// this element entirely. Here it replaces neither — it decorates what the
    /// compositor composited.
    pub fn offloads_after_band(&self) -> bool {
        compositor_background_two_draw_offload::offload::offloads_after_band(
            self.offthread,
            self.pipeline.as_deref(),
        )
    }

    /// Pump the worker for an `AfterBand` bundle and publish whatever decorated
    /// band it has ready. Called INSTEAD of taking the worker's output as the
    /// background — the element still draws its own head band this frame.
    pub fn after_band(&self) -> Option<Dmabuf> {
        self.worker_frame().map(|(buf, _)| buf)
    }

    /// Opt this instance into the off-thread worker, per the live setting.
    ///
    /// THE one place that policy lives. It used to be inline in `TwoSystem`'s
    /// rebuild, which only fires when the slot is empty — so the picker world,
    /// which fills its own slot synchronously during the render pass, never went
    /// through it and rendered its shader inline whatever the setting said.
    /// Every construction site must call this.
    ///
    /// `offthread` is set even when the worker failed to start, so the background
    /// goes absent rather than silently reverting to the inline shader —
    /// reverting would hide the very stall the worker exists to remove.
    ///
    /// GATED ON VULKAN by `engaged()`, not on `enabled` alone. Only the dmabuf
    /// path in `draw.node` ever reads a worker frame, so on GLES an `enabled`
    /// worker would spawn a thread, a second `VkInstance`/`VkDevice` and a
    /// fullscreen ring PER PANE that nothing can consume — while the element
    /// went on drawing the inline shader anyway.
    pub fn attach_worker(&mut self) {
        if compositor_model_environment_background_base::base::get().engaged() {
            self.offthread = true;
            self.worker = compositor_background_two_worker_shared::shared::worker();
        }
    }

    /// Off-thread path: publish this pane's state to the worker and hand back its
    /// newest finished frame.
    ///
    /// The ping does NOT schedule a render — the worker runs at its own rate. It
    /// says "this pane is still live, here is its current camera", and wakes the
    /// worker if it had parked.
    ///
    /// `None` means "draw no background": either there is no worker (the inline
    /// path runs instead) or this pane has not completed a frame yet. Returning
    /// a buffer that was never written would be worse than drawing nothing.
    /// The returned generation is the DAMAGE KEY. While it is unchanged the
    /// background buffer is byte-identical, so the caller must report no damage
    /// and let the compositor skip the region entirely — otherwise a 30fps
    /// background is re-read and re-blended on every compositor frame.
    pub fn worker_frame(&self) -> Option<(Dmabuf, usize)> {
        let worker = self.worker.as_ref()?;
        // NEVER fall back to a shared pane. An instance that gets here unbound was
        // built somewhere that did not stamp its world, and the only key available
        // without one is a key other worlds compute too — which is precisely the
        // cross-world buffer sharing `pane` exists to prevent, arrived at silently.
        // Drawing nothing is what this path already does before a pane's first
        // frame completes; sharing another world's accumulation is not something
        // the picture recovers from.
        let Some(pane) = self.pane.as_ref() else {
            static UNBOUND: std::sync::Once = std::sync::Once::new();
            UNBOUND.call_once(|| {
                warn!(
                    "background: parallax instance reached the worker unbound \
                     (world={:?}); drawing no background rather than sharing a pane",
                    self.world
                )
            });
            return None;
        };
        let size = (self.output_size.0 as u32, self.output_size.1 as u32);
        if size.0 == 0 || size.1 == 0 {
            return None;
        }
        let pan = (
            if self.invert_pan_x { -self.pan.0 } else { self.pan.0 },
            if self.invert_pan_y { -self.pan.1 } else { self.pan.1 },
        );
        // `time` here is discarded — the worker stamps its own, since its output
        // rate does not track the rate at which we signal it.
        let (_, vk) = compositor_background_two_draw_motion::uniforms(
            0.0, self.motion.lock_amount, pan, self.motion.flow_offset,
            self.motion.velocity, self.zoom, self.output_size, &self.params, self.srgb);
        worker.ping(pane, DrawRequest {
            uniforms: vk,
            params: self.params,
            module: self.vulkan.clone(),
            // Only a fully offloadable graph travels; anything else kept the
            // element on the inline path (`worker_can_render`).
            pipeline: self.pipeline.clone(),
            optimized: self.optimized,
            size,
            refresh: self.refresh,
            serial: self.serial,
            regions: self.regions,
            world_set: self.world_set.clone(),
            content_share: self.content_share.clone(),
            windows_share: self.windows_share.clone(),
        });
        worker.latest(pane)
    }

    /// Call right before draw to splice the previous pan and bump damage.
    pub fn update(&mut self) {
        self.motion.tick(self.pan, self.lock_time.is_some());
        self.commit.increment();
    }
    /// Put the "locked" (distant) look on IMMEDIATELY, skipping the 1s
    /// `lock_amount` ramp in `Motion::tick`. For an overlay that owns its own
    /// entry transition (the world picker), that ramp is a second, competing
    /// background animation rather than part of the look.
    pub fn snap_locked(&mut self) {
        self.lock_time = Some(Instant::now());
        self.motion.lock_amount = 1.0;
    }
    /// Build this clone's pane key — but only when there is a worker to key.
    ///
    /// The key exists for the off-thread path and nothing else: `worker_frame`
    /// pings and samples with it, and the pointer warp reads that pane's readback
    /// through it. With triple buffering off there is no worker, nothing will ever
    /// look it up, and building one would be an `Arc` bump per pane per frame on
    /// the path that must cost what it did before this feature existed.
    fn pane_for(
        &self,
        make: impl FnOnce(uuid::Uuid) -> compositor_background_two_worker_key::key::PaneKey,
    ) -> Option<compositor_background_two_worker_key::key::PaneKey> {
        self.worker.as_ref()?;
        self.world.map(make)
    }

    /// Take what the renderer produced for THIS output, for the world this
    /// instance belongs to.
    ///
    /// Bound here rather than stamped where the element leaves the world, because
    /// the world does not know which output is about to draw it — and the
    /// geometry is per output: `collect` normalises every rect to the pass's own
    /// extent, so a set from the other monitor describes the same window in the
    /// wrong UV space.
    pub fn bind_frame(
        &mut self,
        frame: compositor_pipeline_world_system_base::base::OutputFrame,
    ) {
        self.world_set = frame.world_set;
        self.content_share = frame.content_share;
        self.windows_share = frame.windows_share;
    }

    /// Bind the identity an OVERLAY pass has to supply for itself.
    ///
    /// The orchestration scene sets these in its region loop (`bind_pane`), but
    /// the picker and the lock screen build their own plans and so never did —
    /// leaving the constructor's placeholders: pane 0, the fallback refresh, and
    /// a `serial` frozen at 0.
    ///
    /// Frozen is the damaging one. Under `Cadence::Vblank` the worker's rate gate
    /// asks `serial - last_serial >= need`, which is false FOREVER once the
    /// serial stops advancing — so the only thing still admitting a render is the
    /// stall rescue at `min_interval * 3`, a timeout rather than a rate. That is
    /// what an overlay's backdrop was actually running at.
    ///
    /// `namespace` separates overlays that each draw a full-output backdrop: the
    /// picker and the lock screen would otherwise collide with each other and
    /// with the world behind them, all three being `(output, region 0)`. It is a
    /// `Region::Overlay`, NOT a decoration on the output string — the output half
    /// of the key is what a monitor removal matches against, so an overlay that
    /// disguised its output could never be retired by unplugging it.
    pub fn bind_overlay(
        &mut self,
        namespace: &'static str,
        output: &std::sync::Arc<str>,
        refresh: std::time::Duration,
        serial: u64,
    ) {
        self.pane = self.pane_for(|w| {
            compositor_background_two_worker_key::key::PaneKey::overlay(output, w, namespace)
        });
        self.refresh = refresh;
        self.serial = serial;
        self.regions = 1;
    }

    /// Rebind a clone to a viewport pane (render rect + pane camera + distinct id).
    ///
    /// Takes the output and region rather than a finished key, so the derivation
    /// lives here beside `bind_overlay` and the scene builder cannot compute a
    /// key that disagrees with the one the worker is keyed on.
    #[allow(clippy::too_many_arguments)]
    pub fn bind_pane(&mut self, offset: (i32, i32), size: (f32, f32), pan: (f32, f32), zoom: f32, id: Id, output: &std::sync::Arc<str>, region: usize, refresh: std::time::Duration, serial: u64, regions: usize) {
        self.offset = offset; self.output_size = size; self.pan = pan; self.zoom = zoom; self.id = id;
        self.pane = self.pane_for(|w| {
            compositor_background_two_worker_key::key::PaneKey::viewport(output, w, region)
        });
        self.refresh = refresh; self.serial = serial; self.regions = regions;
    }
}

impl Element for ParallaxBackground {
    fn id(&self) -> &Id { &self.id }
    fn current_commit(&self) -> CommitCounter { self.commit }
    fn src(&self) -> Rectangle<f64, Buffer> { Rectangle::from_loc_and_size((0.0, 0.0), (1.0, 1.0)) }
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::from_loc_and_size(self.offset, (self.output_size.0 as i32, self.output_size.1 as i32))
    }
    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> { Point::from(self.offset) }
    fn transform(&self) -> Transform { Transform::Normal }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit != Some(self.commit) {
            vec![self.geometry(scale)].into_iter().collect()
        } else {
            DamageSet::default()
        }
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> { OpaqueRegions::default() }
    fn alpha(&self) -> f32 { 1.0 }
    fn kind(&self) -> Kind { Kind::Unspecified }
    fn is_framebuffer_effect(&self) -> bool { false }
}

impl<R: SceneDispatch> RenderElement<R> for ParallaxBackground {
    fn draw(
        &self,
        frame: &mut <R as RendererSuper>::Frame<'_, '_>,
        _src: Rectangle<f64, Buffer>, dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>], _cache: Option<&UserDataMap>,
    ) -> Result<(), <R as RendererSuper>::Error> {
        // The INLINE path. The pipeline op this draw produces carries `owns` by
        // itself, and the element's own `ElementMeta` carries it too — both belong
        // to this frame, so there is nothing to withdraw and nothing to go stale.
        //
        // The decorated band needs no withdrawal here: it is taken by the frame
        // that presents it, so it lapses unless `pump_after_band` re-publishes.
        // Withdrawing from this path was never enough anyway — a fully-offloaded
        // bundle never reaches `draw()` at all.
        // The SHARED clock, not this element's own start: a window timestamp is
        // published on it, and `t - times.life[i].x` is only an age if both sides
        // measure from the same origin. It also puts every pane's animation in
        // phase, which a per-instance `start_time` never was.
        let time = compositor_pipeline_abi_clock_base::base::now();
        // Per-world pan inversion: flip the pan feeding the shader on each axis.
        let pan = (
            if self.invert_pan_x { -self.pan.0 } else { self.pan.0 },
            if self.invert_pan_y { -self.pan.1 } else { self.pan.1 },
        );
        let (uniforms, vk) = compositor_background_two_draw_motion::uniforms(
            time, self.motion.lock_amount, pan, self.motion.flow_offset,
            self.motion.velocity, self.zoom, (dst.size.w as f32, dst.size.h as f32), &self.params, self.srgb);
        let src = Rectangle::from_loc_and_size((0.0, 0.0), (dst.size.w as f64, dst.size.h as f64));
        let size = Size::from((dst.size.w, dst.size.h));
        // Multipass bundle: run the whole graph (Vulkan). GLES ignores the
        // pipeline field and falls through to the single-pass paths below.
        if let Some(cp) = &self.pipeline {
            return R::draw_pixel_program(
                frame, self.program.as_ref(), src, dst, size, damage, 1.0, &uniforms,
                compositor_pipeline_build_seam_base::base::multipass_pass(cp, &vk, &self.params),
            );
        }
        match &self.vulkan {
            Some(m) => R::draw_pixel_program(
                frame, self.program.as_ref(), src, dst, size, damage, 1.0, &uniforms,
                compositor_background_two_draw_select::loaded_pass(m, &vk, &self.params),
            ),
            None => {
                let pass = compositor_background_two_draw_vulkan::vulkan::ParallaxPass::new(
                    &vk, &self.params, self.optimized);
                R::draw_pixel_program(
                    frame, self.program.as_ref(), src, dst, size, damage, 1.0, &uniforms, pass.pass(),
                )
            }
        }
    }
}

/// The worker-global identity of one background draw target. Re-exported for the
/// callers that hold a key; the derivation itself is `bind_pane`/`bind_overlay`,
/// which are the only places that build one.
pub use compositor_background_two_worker_key::key::{PaneKey, Region};
