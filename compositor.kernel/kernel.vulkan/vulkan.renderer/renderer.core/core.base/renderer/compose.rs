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
    /// The pass's full extent. Needed because an "unscissored" draw is not a
    /// thing at the command-buffer level — the scissor is pipeline-dynamic STATE,
    /// so a draw without one inherits whatever the previous draw set. See
    /// [`Self::scissored`].
    pub extent: (u32, u32),
    /// Whether to scissor each draw to its damage rects. False forces one
    /// full-extent draw.
    pub damaged: bool,
    /// Whether the engine leaves the world band to the bundle this frame.
    pub skip_windows: bool,
    /// Where the WORLD band ends and the SCREEN band begins. See the caller.
    pub split_at: usize,
}


/// The parts of `scissors` that lie inside `clip` — the intersection of two rect
/// lists, used to constrain the opaque pass to the damage actually being redrawn.
///
/// Both lists are small (a handful of damage rects against a handful of opaque
/// rects), so the quadratic walk is cheaper than any structure that would avoid it.
fn clip_rects(scissors: &[vk::Rect2D], clip: &[vk::Rect2D]) -> Vec<vk::Rect2D> {
    let mut out = Vec::new();
    for s in scissors {
        let (sx0, sy0) = (s.offset.x, s.offset.y);
        let (sx1, sy1) = (sx0 + s.extent.width as i32, sy0 + s.extent.height as i32);
        for c in clip {
            let (cx0, cy0) = (c.offset.x, c.offset.y);
            let (cx1, cy1) = (cx0 + c.extent.width as i32, cy0 + c.extent.height as i32);
            let x0 = sx0.max(cx0);
            let y0 = sy0.max(cy0);
            let x1 = sx1.min(cx1);
            let y1 = sy1.min(cy1);
            if x1 > x0 && y1 > y0 {
                out.push(vk::Rect2D {
                    offset: vk::Offset2D { x: x0, y: y0 },
                    extent: vk::Extent2D { width: (x1 - x0) as u32, height: (y1 - y0) as u32 },
                });
            }
        }
    }
    out
}

impl Composer<'_> {
    /// Scissor to every rect and draw, ALWAYS — even on a full redraw.
    ///
    /// [`Self::scissored`] treats its rects as a damage optimization and drops them
    /// when the whole frame is being repainted. That is right for damage and wrong
    /// for a draw whose rects are part of its MEANING, like the opaque pass: there,
    /// ignoring them would paint the whole quad opaque.
    fn scissored_strict(&self, cmd: vk::CommandBuffer, scissors: &[vk::Rect2D], draw: &dyn Fn()) {
        for s in scissors {
            unsafe {
                self.dev.device.cmd_set_scissor(cmd, 0, std::slice::from_ref(s));
            }
            draw();
        }
    }

    /// Run each draw once per damage rect under a scissor, instead of once over
    /// the whole target. On a full redraw (or the offscreen path, which forces
    /// one), a single draw under the pass's FULL EXTENT.
    ///
    /// That last word is the fix. This used to just call `draw()` with no
    /// `cmd_set_scissor` at all, on the reasoning that a full redraw wants no
    /// scissor — but the scissor is dynamic pipeline STATE, not a per-draw
    /// argument, so "no scissor" means "whatever the last draw left set". And
    /// something always has: [`Self::scissored_strict`] sets one per rect for the
    /// opaque pass and never restores it.
    ///
    /// So on the offscreen path — which forces `damaged = false`, and which only a
    /// bundle with an after-content pass turns on — every element after the first
    /// opaque replacement was drawn clipped to the PREVIOUS element's opaque rect.
    /// A window placed over another rendered only inside that other window's
    /// geometry; one over empty background vanished entirely; and it tracked pan
    /// and zoom, because the inherited rect is another element's screen position.
    fn scissored(&self, cmd: vk::CommandBuffer, scissors: &[vk::Rect2D], draw: &dyn Fn()) {
        if !self.damaged {
            self.scissor_full(cmd);
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

    /// Reset the scissor to the whole pass. The state the pass begins in, restated
    /// wherever a draw means "all of it" — because nothing else restores it.
    fn scissor_full(&self, cmd: vk::CommandBuffer) {
        let full = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: self.extent.0, height: self.extent.1 },
        };
        unsafe {
            self.dev.device.cmd_set_scissor(cmd, 0, std::slice::from_ref(&full));
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
            DrawOp::Textured { quad, tex_w, tex_h, scissors, opaque, .. } => {
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
                    // Every WORLD element — which is every client window — takes this
                    // path whenever AA/FSR is on, so it needs the same opaque handling
                    // as the plain pipeline below. Blend the quad, then replace the
                    // declared-opaque parts.
                    self.scissored(cmd, scissors, &|| aa.draw(dev, cmd, set, push, false));
                    let aa_opaque = if self.damaged {
                        clip_rects(scissors, opaque)
                    } else {
                        opaque.clone()
                    };
                    if !aa_opaque.is_empty() {
                        self.scissored_strict(cmd, &aa_opaque, &|| {
                            aa.draw(dev, cmd, set, push, true)
                        });
                    }
                } else {
                    self.scissored(cmd, scissors, &|| {
                        compositor_kernel_vulkan_element_texture_base::texture::draw(
                            dev, pipelines, cmd, set, *quad, false,
                        );
                    });
                    // Then REPLACE the parts the surface declared opaque. Done as a
                    // second scissored pass rather than by splitting the first: the
                    // opaque region is an arbitrary rect list clipped against the
                    // damage rects, and subtracting one list from the other to get the
                    // blended remainder is both fiddly and pointless here — redrawing
                    // the opaque part over itself costs one extra pass over a region
                    // that is opaque by definition, and cannot be wrong.
                    //
                    // This is what makes the damage tracker's decision safe: it has
                    // already removed these rects from `clear_rects` on the strength of
                    // the client's promise, so something has to actually paint them.
                    //
                    // `scissored_strict`, not `scissored`: the latter ignores its rects
                    // and draws once unscissored on a full redraw, which here would
                    // paint the WHOLE window opaque and destroy any genuinely
                    // translucent part of it.
                    let opaque_scissors = if self.damaged {
                        clip_rects(scissors, opaque)
                    } else {
                        opaque.clone()
                    };
                    if !opaque_scissors.is_empty() {
                        self.scissored_strict(cmd, &opaque_scissors, &|| {
                            compositor_kernel_vulkan_element_texture_base::texture::draw(
                                dev, pipelines, cmd, set, *quad, true,
                            );
                        });
                    }
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
