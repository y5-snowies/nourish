//! Recording one composite: how each `DrawOp` is drawn, and which ops make up
//! each band.
//!
//! Split out of `submit_frame` because it is the part a reviewer has to follow
//! closely and it was buried in the middle of a function that also resolves
//! bundle requirements, allocates descriptor sets and drives the offscreen path.
//! Nothing here is new behaviour — it is the same per-op draw and the same band
//! iteration, addressed through one struct instead of a stack of closures.
//!
//! # Why a struct rather than closures
//!
//! The three band callbacks (`compose`, `compose_windows`, `compose_screen`) all
//! need the same eighteen values and all defer to the same `draw_one`. As
//! closures that meant each one capturing the set independently, which is what
//! made the diff unreadable: moving one line moved the borrow set of three
//! callbacks. Here the captures are named once, and the caller hands the methods
//! to `record_composition` as thin closures.

use crate::frame::DrawOp;
use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_pipeline_composite_base::composite::{AaComposite, CompositePipelines};
use compositor_kernel_vulkan_pipeline_fullscreen_base::fullscreen::FullscreenPass;
use compositor_pipeline_execute_graph_base::graph::GraphExec;
use std::collections::HashMap;

/// The world anti-aliasing parameters, which travel together and are read only by
/// the one `AaPush` below.
#[derive(Clone, Copy)]
pub(super) struct Aa {
    pub taps: u32,
    pub spread: f32,
    pub sharpen: f32,
    pub lod_bias: f32,
    pub easu: bool,
    pub rcas: bool,
    pub rcas_strength: f32,
}

/// Everything recording this frame's composite needs.
pub(super) struct Composer<'a> {
    pub dev: &'a VulkanDevice,
    pub pipelines: &'a CompositePipelines,
    pub shader_passes: &'a HashMap<(u64, vk::Format), FullscreenPass>,
    pub graph: &'a GraphExec,
    pub aa: Option<&'a AaComposite>,
    pub ops: &'a [DrawOp],
    /// Lockstep with `ops`: the descriptor set a textured op draws with.
    pub sets: &'a [Option<vk::DescriptorSet>],
    /// Lockstep with `ops`: whether this op takes the AA path.
    pub aa_op: &'a [bool],
    pub clear_rects: &'a [vk::Rect2D],
    pub demand: &'a super::worldset::Demand,
    pub aa_params: Aa,
    /// The frame counter, for the graph's per-frame parity: a persistent target
    /// is a pair of images and this is what says which of them is being written.
    pub tick: u64,
    pub use_hdr: bool,
    pub format: vk::Format,
    /// Whether to scissor each draw to its damage rects. False forces one
    /// unscissored draw under the pass's full extent.
    pub damaged: bool,
    /// Whether the engine leaves the world band to the bundle this frame.
    pub skip_windows: bool,
    /// Where the WORLD band ends and the SCREEN band begins. See the caller.
    pub split_at: usize,
}

impl Composer<'_> {
    /// Run each draw once per damage rect under a scissor, instead of once over
    /// the whole target. On a full redraw (or the offscreen path, which forces
    /// one), a single unscissored draw under `begin`'s full extent.
    fn scissored(&self, cmd: vk::CommandBuffer, scissors: &[vk::Rect2D], draw: &dyn Fn()) {
        if !self.damaged {
            draw();
            return;
        }
        for s in scissors {
            unsafe {
                self.dev.device.cmd_set_scissor(cmd, 0, std::slice::from_ref(s));
            }
            draw();
        }
    }

    /// Draw one op.
    ///
    /// `skip_win` is a PARAMETER, not a field: the world band honours
    /// `windows: pipeline` and omits client windows, while the pass that fills the
    /// `windows` LAYER must draw them regardless. Making it a field once made the
    /// two mutually exclusive — and they are precisely the combination that gives
    /// a pipeline separated background and window layers to composite itself.
    fn draw_one(&self, cmd: vk::CommandBuffer, i: usize, skip_win: bool) {
        let (dev, pipelines) = (self.dev, self.pipelines);
        match &self.ops[i] {
            DrawOp::Solid { quad, scissors } => {
                self.scissored(cmd, scissors, &|| {
                    compositor_kernel_vulkan_element_solid_base::solid::draw(
                        dev, pipelines, cmd, *quad,
                    );
                });
            }
            DrawOp::ShaderPass { sdr, hdr, scissors } => {
                let v = if self.use_hdr { hdr.as_ref().unwrap_or(sdr) } else { sdr };
                if let Some(fp) = self.shader_passes.get(&(v.id, self.format)) {
                    self.scissored(cmd, scissors, &|| fp.draw(dev, cmd, &v.push));
                }
            }
            // The graph's output pass is fullscreen and carries no per-op rects;
            // scissor it to the frame damage so a partial redraw does not repaint
            // the whole background (and so the pass never inherits a previous op's
            // scissor).
            DrawOp::Pipeline(p) => {
                self.scissored(cmd, self.clear_rects, &|| self.graph.draw_output(dev, cmd, p, self.tick));
            }
            // The pipeline owns this drawable's pixels — the engine collected its
            // rect/src/view into the world set but must not draw it.
            DrawOp::Textured { meta, .. } if skip_win && self.demand.claim.owns(meta) => {}
            DrawOp::Textured { quad, tex_w, tex_h, scissors, .. } => {
                let set = self.sets[i].expect("textured op has a set");
                if self.aa_op[i] {
                    let aa = self.aa.expect("aa pipeline present for aa op");
                    let p = self.aa_params;
                    let push = compositor_kernel_vulkan_pipeline_composite_base::composite::AaPush {
                        dst: quad.dst,
                        src: quad.src,
                        color: quad.color,
                        params: [p.taps as f32, p.spread, p.sharpen, p.lod_bias],
                        params2: [
                            if p.easu { 1.0 } else { 0.0 },
                            if p.rcas { p.rcas_strength } else { 0.0 },
                            *tex_w as f32,
                            *tex_h as f32,
                        ],
                    };
                    self.scissored(cmd, scissors, &|| aa.draw(dev, cmd, set, push));
                } else {
                    self.scissored(cmd, scissors, &|| {
                        compositor_kernel_vulkan_element_texture_base::texture::draw(
                            dev, pipelines, cmd, set, *quad,
                        );
                    });
                }
            }
        }
    }

    /// The WORLD band: everything up to the split.
    pub fn compose(&self, cmd: vk::CommandBuffer) {
        for i in 0..self.split_at {
            self.draw_one(cmd, i, self.skip_windows);
        }
    }

    /// The world band alone, for the `windows` layer: the same per-op draw as the
    /// full band, minus the background. Ordering is preserved, so overlapping
    /// drawables composite as they do on screen.
    ///
    /// `is_world()`, not `is_window()`. The layer is a stand-in for "what the
    /// engine draws over the background", and a version of that which silently
    /// omits iced-world panels has the same hole as the old window set: a bundle
    /// that rebuilds the band from this layer would drop them.
    pub fn compose_windows(&self, cmd: vk::CommandBuffer) {
        for i in 0..self.split_at {
            if matches!(&self.ops[i], DrawOp::Textured { meta, .. } if meta.is_world()) {
                self.draw_one(cmd, i, false);
            }
        }
    }

    /// The SCREEN band: everything after the split, drawn ON TOP of the
    /// post-processed world so a vignette or glass never darkens the UI or cursor.
    pub fn compose_screen(&self, cmd: vk::CommandBuffer) {
        for i in self.split_at..self.ops.len() {
            self.draw_one(cmd, i, self.skip_windows);
        }
    }
}
