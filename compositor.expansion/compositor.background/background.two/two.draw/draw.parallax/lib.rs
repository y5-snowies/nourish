//! `ParallaxBackground`: the infinite-canvas background render element.
use compositor_background_two_draw_motion::Motion;
use compositor_background_two_shader_spirv::VulkanModule;
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
    motion: Motion,
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
    ) -> Self {
        let (program, vulkan, params, shader_error) =
            compositor_background_two_draw_select::build(renderer, selection, params_override);
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
            invert_pan_x: false, invert_pan_y: false, srgb: false, motion: Motion::new(),
        }
    }
    /// Call right before draw to splice the previous pan and bump damage.
    pub fn update(&mut self) {
        self.motion.tick(self.pan, self.lock_time.is_some());
        self.commit.increment();
    }
    /// Rebind a clone to a viewport pane (render rect + pane camera + distinct id).
    pub fn bind_pane(&mut self, offset: (i32, i32), size: (f32, f32), pan: (f32, f32), zoom: f32, id: Id) {
        self.offset = offset; self.output_size = size; self.pan = pan; self.zoom = zoom; self.id = id;
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
                let pass = compositor_background_two_draw_vulkan::vulkan::ParallaxPass::new(&vk, &self.params);
                R::draw_pixel_program(
                    frame, self.program.as_ref(), src, dst, size, damage, 1.0, &uniforms, pass.pass(),
                )
            }
        }
    }
}
