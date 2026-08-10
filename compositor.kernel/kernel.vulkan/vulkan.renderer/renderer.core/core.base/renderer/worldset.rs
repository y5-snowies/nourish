//! Collecting the frame's world drawables for a shader bundle, and deciding how
//! much of the band the engine must therefore NOT draw.
//!
//! Split out of `submit_frame` because it is one self-contained concern with a lot
//! of rationale attached, and leaving it inline pushed the composite path — the
//! part a reviewer actually has to follow — under two hundred lines of collection
//! code it does not interact with.
//!
//! # The one predicate
//!
//! Whatever the engine leaves out, the bundle must draw, and vice versa. A gap
//! either way is an ordering bug: content drawn twice, or world content floating
//! above windows it belongs under. `Claim::owns_op` and `WorldSet::owned` are the
//! two halves and they are derived from the same value here so they cannot drift.

use ash::vk;
use compositor_pipeline_abi_worldset_base::base::{Facts, Kind, Own, Source, WorldSet};
use crate::frame::DrawOp;
use compositor_orchestration_draw_dispatch_frame::{ElementMeta};
use compositor_pipeline_abi_seam_base::base::{Requirement, Requires};

/// How much of the world band this frame's bundle takes over, and the test the
/// engine applies to each drawable because of it.
pub struct Claim {
    own: Own,
    /// Whether the shader receives the WHOLE band rather than client windows only.
    ///
    /// Two different reasons to get it. `Own::World` is "I draw it, so the engine
    /// must not". `requires.whole_band()` is "the engine still draws it, but I
    /// re-composite what I find and must not miss any of it" — `glass`, which would
    /// otherwise paint a window back over a placeholder stacked on top of it.
    pub whole_band: bool,
}

impl Claim {
    /// §8d — the bundle composites part of the band itself from the world-texture
    /// array, so the engine must leave exactly that part out. Without this the
    /// pipeline would draw its transformed copy ON TOP of the engine's own
    /// axis-aligned blit of the same drawable, and effects that MOVE or hide one
    /// could never work: `content` would already have destroyed what was behind it.
    pub fn owns(&self, meta: &ElementMeta) -> bool {
        match self.own {
            Own::None => false,
            Own::Windows => meta.is_window(),
            Own::World => meta.is_world(),
        }
    }

    /// Whether the engine skips the world band entirely this frame.
    pub fn suppresses_band(&self) -> bool {
        self.own != Own::None
    }

    pub fn own(&self) -> Own {
        self.own
    }
}

/// Everything this frame's bundle asks the engine for, resolved in one place.
///
/// A pure function of the ops plus the shared requirement slot — no device, no
/// `&mut self` — so `submit_frame` reads one call instead of sixty lines of
/// interleaved predicates. Every field is `false`/empty with no bundle loaded,
/// which is what keeps the stock composite path the old code rather than the old
/// code threaded with conditionals that happen not to fire.
pub struct Demand {
    /// The union the drawn world's bundle declared. Taken from [`Facts`] rather
    /// than from `ops`, because an OFFLOADED pipeline leaves no op in this frame
    /// and is precisely the case that still needs the world set collected.
    pub requires: Requires,
    /// How much of the world band the bundle takes over.
    pub claim: Claim,
    /// A pipeline has after-content passes, so the scene must be composited
    /// offscreen into `content` for them to sample.
    pub has_after: bool,
    /// The persistent previous-frame image is wanted (filled from `content`).
    pub keep_history: bool,
    /// The window layer is wanted (composited alongside `content`).
    pub keep_windows: bool,
}

impl Demand {
    pub fn resolve(ops: &[DrawOp], facts: Facts) -> Demand {
        let requires = Requires::from_bits(facts.requires);
        Demand {
            requires,
            claim: claim(
                ops.iter()
                    .find_map(|o| match o {
                        DrawOp::Pipeline(p) if p.owns != Own::None => Some(p.owns),
                        _ => None,
                    })
                    // Offloaded: the graph ran on the worker and leaves no op here,
                    // so fall back to what the loaded bundle declared. Without it
                    // the engine blits the band the worker already drew and every
                    // window appears twice. Gated on the frame actually CARRYING a
                    // band: a worker that failed to start lowers no element, and
                    // suppressing windows for a band that is not there would leave
                    // the desktop blank rather than doubled.
                    .or_else(|| {
                        let band = ops.iter().any(
                            |o| matches!(o, DrawOp::Textured { meta, .. } if meta.is_background()),
                        );
                        Some(facts.owns_band()).filter(|o| band && *o != Own::None)
                    })
                    // The picker, the lock and the overview put a picture up that
                    // the live bundle did not draw, so neither arm above speaks for
                    // this frame. `owns_band` already returns `None` there; an
                    // INLINE bundle's op carries its own claim and does not go
                    // through it. See `set::band_suppressed`.
                    .filter(|_| !facts.suppressed),
                ops.iter().any(|o| matches!(o, DrawOp::Pipeline(p) if p.requires.whole_band())),
            ),
            has_after: ops.iter().any(|o| matches!(o, DrawOp::Pipeline(p) if p.has_after())),
            keep_history: requires.has(Requirement::PreviousFrame),
            keep_windows: requires.has(Requirement::WindowLayer),
        }
    }

    /// Whether the composite must go through the offscreen `content` path.
    ///
    /// HDR keeps the direct path regardless: its own composite already owns the
    /// output and the two cannot both drive it.
    pub fn offscreen(&self, use_hdr: bool) -> bool {
        (self.has_after || self.keep_history || self.keep_windows) && !use_hdr
    }

    /// Whether the engine must leave the world band to the bundle this frame.
    pub fn skip_windows(&self) -> bool {
        self.claim.suppresses_band()
    }

    /// Collect and publish this frame's world set from `ops`. See [`collect`].
    pub fn gather(&self, ops: &[DrawOp], extent: (u32, u32)) -> Collected {
        collect(
            self.requires,
            extent,
            ops.iter().filter_map(|op| match op {
                DrawOp::Textured { world_rect: Some(r), view, quad, source, meta, .. } => {
                    Some(Drawable {
                        rect: *r,
                        src: quad.src,
                        alpha: quad.color[3],
                        is_window: meta.is_window(),
                        flags: meta.flags,
                        times: meta.times,
                        view: *view,
                        source,
                    })
                }
                _ => None,
            }),
        )
    }
}

/// Resolve the claim for this frame.
///
/// `op_owns` is the `owns` carried by the graph's op in THIS frame, so a frame
/// with no pipeline op makes no claim at all.
pub fn claim(op_owns: Option<Own>, whole_band_op: bool) -> Claim {
    let own = op_owns.unwrap_or(Own::None);
    Claim { whole_band: own == Own::World || whole_band_op, own }
}

/// The frame's world drawables, index-aligned: entry `i` is one drawable.
#[derive(Default)]
pub struct Collected {
    pub rects: Vec<[f32; 4]>,
    pub srcs: Vec<[f32; 4]>,
    pub kinds: Vec<Kind>,
    pub alphas: Vec<f32>,
    pub flags: Vec<u32>,
    /// Empty unless the bundle declared `window_times` — that requirement gates
    /// the gather, not just the upload.
    pub times: Vec<[f32; 12]>,
    pub views: Vec<vk::ImageView>,
    pub sources: Vec<Source>,
}

/// What the shader is actually bound, which is a SUBSET of what was published
/// unless the bundle takes the whole band. Otherwise panels are filtered out and
/// the indices close up, so a bundle written against the old window-only set sees
/// exactly the array it always saw — same entries, same indices, no change needed.
pub struct Bound {
    pub rects: Vec<[f32; 4]>,
    pub srcs: Vec<[f32; 4]>,
    pub meta: Vec<[f32; 4]>,
    /// The `Times` UBO's three arrays, split out of the twelve-lane rows. Empty
    /// when the bundle did not ask for them, which is what `set_world_times`
    /// skips on.
    pub life: Vec<[f32; 4]>,
    pub state: Vec<[f32; 4]>,
    pub drag: Vec<[f32; 4]>,
    pub views: Vec<vk::ImageView>,
}

impl Collected {
    pub fn bind(&self, claim: &Claim) -> Bound {
        let owned: Vec<usize> = (0..self.kinds.len())
            .filter(|&i| claim.whole_band || self.kinds[i] == Kind::Window)
            .collect();
        let pick = |v: &[[f32; 4]]| owned.iter().map(|&i| v[i]).collect::<Vec<_>>();
        Bound {
            rects: pick(&self.rects),
            srcs: pick(&self.srcs),
            // Through the shared packer, never a literal here: the worker builds
            // the same array from the same values in `two.worker`, and a layout
            // written twice is a layout that eventually differs.
            meta: owned
                .iter()
                .map(|&i| {
                    compositor_pipeline_abi_descriptor_base::base::attrs(
                        self.kinds[i] as u8,
                        self.alphas[i],
                        self.flags[i],
                    )
                })
                .collect(),
            life: owned
                .iter()
                .filter(|&&i| i < self.times.len())
                .map(|&i| [self.times[i][0], self.times[i][1], self.times[i][2], self.times[i][3]])
                .collect(),
            state: owned
                .iter()
                .filter(|&&i| i < self.times.len())
                .map(|&i| [self.times[i][4], self.times[i][5], self.times[i][6], self.times[i][7]])
                .collect(),
            drag: owned
                .iter()
                .filter(|&&i| i < self.times.len())
                .map(|&i| [self.times[i][8], self.times[i][9], self.times[i][10], self.times[i][11]])
                .collect(),
            views: owned.iter().map(|&i| self.views[i]).collect(),
        }
    }
}

/// One world drawable as the composite recorded it: its physical dst rect, its
/// crop, its view and how a second device could reach its pixels.
pub struct Drawable<'a> {
    pub rect: [f32; 4],
    pub src: [f32; 4],
    pub alpha: f32,
    pub is_window: bool,
    /// `window.descriptor` bits, tagged onto the element at scene assembly and
    /// carried here on its `ElementMeta`.
    pub flags: u32,
    /// Its window's timestamp row, same provenance.
    pub times: [f32; 12],
    pub view: vk::ImageView,
    pub source: &'a Source,
}

/// Collect this frame's world drawables.
///
/// Gated on the bundle REQUIRING them. Unconditional, this was six allocations,
/// an O(ops) pass and an `Arc` clone per world drawable on every frame of a
/// desktop with no bundle loaded.
///
/// Collected EVERY frame the requirement holds, not only when this frame carries
/// a pipeline op: an offloaded graph leaves no op, and is exactly the case whose
/// worker still needs the geometry.
///
/// Returned, never published. It used to go into a process-global slot that
/// anything could read — which is how one world's drawables reached another
/// world's shader. The frame driver hands this to the world that was drawn, and
/// the two consumers (the pointer warp, and the worker via the element) both take
/// it from there.
///
/// NOT truncated here. `WorldSet::truncate_to_max` caps at the shader array
/// length and says so; capping twice with only one of them logging is how a
/// dropped tail becomes unexplainable.
pub fn collect<'a>(
    requires: Requires,
    extent: (u32, u32),
    drawables: impl Iterator<Item = Drawable<'a>>,
) -> Collected {
    let mut c = Collected::default();
    if !requires.world_set() {
        return c;
    }
    let (ew, eh) = (extent.0 as f32, extent.1 as f32);
    for d in drawables {
        c.rects.push([d.rect[0] / ew, d.rect[1] / eh, d.rect[2] / ew, d.rect[3] / eh]);
        c.srcs.push(d.src);
        c.kinds.push(if d.is_window { Kind::Window } else { Kind::Panel });
        c.alphas.push(d.alpha);
        c.flags.push(d.flags);
        if requires.times() {
            c.times.push(d.times);
        }
        c.views.push(d.view);
        c.sources.push(d.source.clone());
    }
    c
}

impl Collected {
    /// The transportable half — everything but the `ImageView`s, which are this
    /// device's own handles and mean nothing to the worker or the warp.
    pub fn to_world_set(&self) -> WorldSet {
        let mut w = WorldSet {
            rects: self.rects.clone(),
            srcs: self.srcs.clone(),
            kinds: self.kinds.clone(),
            alphas: self.alphas.clone(),
            flags: self.flags.clone(),
            times: self.times.clone(),
            sources: self.sources.clone(),
        };
        w.truncate_to_max();
        w
    }
}
