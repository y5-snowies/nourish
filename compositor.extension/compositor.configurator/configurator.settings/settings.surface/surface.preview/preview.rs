//! The parallax preview: an `iced::widget::shader` program running the *selected*
//! shader's WGSL in a wgpu pipeline (rewriting `var<immediate>` push constants to
//! a uniform buffer, since iced's wgpu device has no push constants), driven by
//! the current `@prop` params + the widget's own pan/zoom, animating off a clock.
use compositor_configurator_settings_surface_message::message::SettingsMessage;
use iced_core::{mouse, Event, Rectangle};
use iced_widget::shader::{self, Viewport};
use iced_wgpu::wgpu;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Instant;

/// The preview widget program; rebuilt each `view()` with the live source+params.
#[derive(Clone)]
pub struct ParallaxPreview {
    /// The WGSL source of the shader to preview (built-in or the selected bundle).
    pub source: String,
    pub params: [f32; 16],
}

/// Per-widget interaction state: accumulated pan/zoom + an active drag.
pub struct State {
    pan: (f32, f32),
    zoom: f32,
    drag: Option<(f32, f32)>,
}
impl Default for State {
    fn default() -> Self {
        Self { pan: (0.0, 0.0), zoom: 1.0, drag: None }
    }
}

impl shader::Program<SettingsMessage> for ParallaxPreview {
    type State = State;
    type Primitive = Primitive;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<SettingsMessage>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = cursor.position_over(bounds) {
                    state.drag = Some((p.x, p.y));
                    return Some(shader::Action::request_redraw().and_capture());
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(prev), Some(p)) = (state.drag, cursor.position()) {
                    state.pan.0 -= (p.x - prev.0) * 2.0;
                    state.pan.1 -= (p.y - prev.1) * 2.0;
                    state.drag = Some((p.x, p.y));
                    return Some(shader::Action::request_redraw());
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => state.drag = None,
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.is_over(bounds) {
                    let dy = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => *y,
                        mouse::ScrollDelta::Pixels { y, .. } => *y / 20.0,
                    };
                    state.zoom = (state.zoom * (1.0 + dy * 0.1)).clamp(0.2, 5.0);
                    return Some(shader::Action::request_redraw().and_capture());
                }
            }
            _ => {}
        }
        Some(shader::Action::request_redraw())
    }

    fn draw(&self, state: &State, _cursor: mouse::Cursor, _bounds: Rectangle) -> Primitive {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.source.hash(&mut h);
        Primitive { source: self.source.clone(), key: h.finish(), params: self.params, pan: state.pan, zoom: state.zoom }
    }
}

#[derive(Debug)]
pub struct Primitive {
    source: String,
    key: u64,
    params: [f32; 16],
    pan: (f32, f32),
    zoom: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    res_zoom_time: [f32; 4],
    pan_flow: [f32; 4],
    lock_alpha: [f32; 4],
    params: [[f32; 4]; 4],
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        pipeline.ensure(device, self.key, &self.source);
        let s = viewport.scale_factor();
        let u = Uniforms {
            res_zoom_time: [bounds.width * s, bounds.height * s, self.zoom, pipeline.start.elapsed().as_secs_f32()],
            pan_flow: [self.pan.0, self.pan.1, 0.0, 0.0],
            lock_alpha: [0.0, 1.0, 0.0, 0.0],
            params: [
                [self.params[0], self.params[1], self.params[2], self.params[3]],
                [self.params[4], self.params[5], self.params[6], self.params[7]],
                [self.params[8], self.params[9], self.params[10], self.params[11]],
                [self.params[12], self.params[13], self.params[14], self.params[15]],
            ],
        };
        let bytes = unsafe {
            std::slice::from_raw_parts((&u as *const Uniforms) as *const u8, std::mem::size_of::<Uniforms>())
        };
        queue.write_buffer(&pipeline.uniforms, 0, bytes);
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(rp) = pipeline.pipelines.get(&self.key) else { return };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("parallax.preview.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(clip_bounds.x as f32, clip_bounds.y as f32, clip_bounds.width as f32, clip_bounds.height as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip_bounds.x, clip_bounds.y, clip_bounds.width, clip_bounds.height);
        pass.set_pipeline(rp);
        pass.set_bind_group(0, &pipeline.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub struct Pipeline {
    pipelines: HashMap<u64, wgpu::RenderPipeline>,
    failed: std::collections::HashSet<u64>,
    layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    start: Instant,
}

impl Pipeline {
    /// Build + cache the render pipeline for `source` (keyed by its hash). The
    /// `var<immediate>` push block is rewritten to a uniform binding. Invalid
    /// WGSL — and valid WGSL whose resource interface doesn't fit the preview's
    /// fixed bind layout — is a FATAL wgpu error, so screen for both with naga
    /// first and skip on failure (cached); the preview shows nothing rather than
    /// crashing.
    fn ensure(&mut self, device: &wgpu::Device, key: u64, source: &str) {
        if self.pipelines.contains_key(&key) || self.failed.contains(&key) {
            return;
        }
        let wgsl = source.replace("var<immediate>", "@group(0) @binding(0) var<uniform>");
        match naga::front::wgsl::parse_str(&wgsl) {
            Ok(module) => {
                let valid = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::empty(),
                )
                .validate(&module)
                .is_ok();
                if !valid || !Self::fits_preview_layout(&module) {
                    self.failed.insert(key);
                    return;
                }
            }
            Err(_) => {
                self.failed.insert(key);
                return;
            }
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("parallax.preview.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(wgsl)),
        });
        let rp = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("parallax.preview.pipeline"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_main"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: self.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, rp);
    }

    /// Whether `module`'s resource interface fits the preview's single fixed
    /// binding: one uniform at group 0 / binding 0, no larger than `Uniforms`.
    /// wgpu validates the bound buffer size against the shader's block at draw
    /// time and treats a mismatch as fatal, so a shader that compiles yet expects
    /// a bigger or differently-placed binding (e.g. the HDR parallax's 96-byte
    /// `HdrPush` against our 80-byte buffer) must be rejected here, not drawn.
    fn fits_preview_layout(module: &naga::Module) -> bool {
        let max = std::mem::size_of::<Uniforms>() as u32;
        module.global_variables.iter().all(|(_, gv)| match &gv.binding {
            None => true,
            Some(rb) if rb.group == 0 && rb.binding == 0 => {
                gv.space == naga::AddressSpace::Uniform
                    && module.types[gv.ty].inner.size(module.to_ctx()) <= max
            }
            Some(_) => false,
        })
    }
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("parallax.preview.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("parallax.preview.bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("parallax.preview.bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("parallax.preview.layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        Self { pipelines: HashMap::new(), failed: std::collections::HashSet::new(), layout, format, uniforms, bind_group, start: Instant::now() }
    }
}
