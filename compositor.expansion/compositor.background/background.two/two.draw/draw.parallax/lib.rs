//! `ParallaxBackground`: the infinite-canvas background render element.
use compositor_background_two_draw_motion::Motion;
use compositor_background_two_shader_spirv::VulkanModule;
use compositor_background_two_worker_base::base::Worker;
use compositor_background_two_worker_signal::signal::DrawRequest;
use smithay::backend::allocator::dmabuf::Dmabuf;
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
    /// Which pane this clone draws. Panes must not share a buffer: each has its
    /// own camera and physical size, so across monitors a shared buffer would be
    /// the wrong resolution and the wrong view for all but one of them.
    pub pane: u64,
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
        let (program, vulkan, params, shader_error) =
            compositor_background_two_draw_select::build(renderer, selection, params_override, optimized);
        Self {
            output_size,
            offset: (0, 0),
            id: Id::new(),
            commit: CommitCounter::default(),
            program,
            vulkan,
            lock_time: None,
            start_time: Instant::now(),
            pan: (0.0, 0.0), zoom: 1.0, params, shader_error,
            invert_pan_x: false, invert_pan_y: false, srgb: false, optimized,
            motion: Motion::new(),
            worker: None, offthread: false, pane: 0, regions: 1,
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
        worker.ping(self.pane, DrawRequest {
            uniforms: vk,
            params: self.params,
            module: self.vulkan.clone(),
            optimized: self.optimized,
            size,
            refresh: self.refresh,
            serial: self.serial,
            regions: self.regions,
        });
        worker.latest(self.pane)
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
    /// Rebind a clone to a viewport pane (render rect + pane camera + distinct id).
    #[allow(clippy::too_many_arguments)]
    pub fn bind_pane(&mut self, offset: (i32, i32), size: (f32, f32), pan: (f32, f32), zoom: f32, id: Id, pane: u64, refresh: std::time::Duration, serial: u64, regions: usize) {
        self.offset = offset; self.output_size = size; self.pan = pan; self.zoom = zoom; self.id = id;
        self.pane = pane; self.refresh = refresh; self.serial = serial; self.regions = regions;
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
        let time = self.start_time.elapsed().as_secs_f32();
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

/// The worker-global identity of one viewport region on one output. Re-exported
/// so the scene builder — which owns the `(output, region)` pair — and the
/// worker agree on one derivation. See `worker.key` for why the region index
/// alone is not enough.
pub use compositor_background_two_worker_key::key::pane_key;
