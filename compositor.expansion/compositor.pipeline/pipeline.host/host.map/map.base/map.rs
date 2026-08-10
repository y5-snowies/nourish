//! A baked displacement map — the GRIDDABLE half of the pointer warp.
//!
//! `hit_inverse(uv, res, params)` is a pure function of position, so between
//! resolution and prop changes it is a fixed field. Sampling it per pointer event
//! re-derives the same answer thousands of times a second; baking it once and
//! looking it up costs a bilinear fetch instead, and the field is smooth, so the
//! interpolation error is bounded by curvature × cell size — invisible at cursor
//! scale.
//!
//! This is only valid because the field IS smooth. A per-drawable transform makes
//! it jump at every drawable edge, and interpolating across that jump is wrong
//! everywhere near it — not to a bounded tolerance, but qualitatively. That half
//! stays pointwise (`Warp::eval_drawable`); the two are the chain's two entry
//! kinds and neither subsumes the other.
//!
//! # Who bakes it
//!
//! The CPU interpreter, today. The natural producer is the GPU — it is already
//! evaluating this function per pixel — but the pointer lives on the CPU, so a
//! GPU-produced map has to be read back, which buys nothing here: the ABI forbids
//! the function from reading textures or bindings, so the GPU has no information
//! the CPU lacks. What a readback WOULD add is a window, after every resolution or
//! prop change, in which the map is not ready and the cursor is silently
//! uncorrected. Baking on the CPU has no such window.
//!
//! Swapping in a GPU producer later changes only [`Map::bake`]: the grid, the
//! sampling and the invalidation are unaffected.

use compositor_pipeline_host_hit_base::hit::{Args, Warp};

/// Cells per edge, RE-EXPORTED from the GPU producer rather than restated.
///
/// Two producers fill this grid — a CPU bake and a GPU render — and the pointer
/// samples whichever it is handed with one indexer. Two independent constants that
/// happened to agree would be a silent reinterpretation of the buffer the moment
/// one moved, so there is one.
///
/// 128² = 16k evaluations per bake (~a millisecond for a barrel on the CPU,
/// nothing on a GPU).
pub const EDGE: usize = compositor_pipeline_execute_warpmap_base::base::EDGE as usize;

/// A baked map plus the inputs it was baked for — carried so the holder can tell
/// when it is stale rather than tracking that separately and getting it wrong.
pub struct Map {
    res: [f32; 2],
    params: [f32; 4],
    /// Source UV per grid node, row-major, `EDGE`×`EDGE`.
    cells: Vec<[f32; 2]>,
}

impl Map {
    /// Bake `warp`'s griddable entry over the unit square. `None` if the bundle
    /// defines no such entry, or if evaluation gave up (see `shader.eval`).
    ///
    /// A single failed node fails the whole bake: a map with one interpolated-over
    /// hole would displace the pointer smoothly toward a value nobody computed,
    /// which is worse than falling back to evaluating per event.
    pub fn bake(warp: &Warp, res: [f32; 2], params: [f32; 4]) -> Option<Map> {
        // A warp that reads the clock describes one instant, so a bake of it is
        // wrong every frame after the one it was taken in. `load_pipeline` refuses
        // that combination outright; this is the belt to that braces.
        if !warp.has_map() || warp.animated() {
            return None;
        }
        let mut cells = Vec::with_capacity(EDGE * EDGE);
        let last = (EDGE - 1) as f64;
        for y in 0..EDGE {
            for x in 0..EDGE {
                let (u, v) = (x as f64 / last, y as f64 / last);
                let (su, sv) = warp.eval((u, v), Args { res, params, time: 0.0 })?;
                cells.push([su as f32, sv as f32]);
            }
        }
        Some(Map { res, params, cells })
    }

    /// Wrap a grid produced elsewhere — the GPU, for an animating warp.
    ///
    /// Same cells, same sampler. The producer is the only thing that differs
    /// between `map_static` and `map`, which is why it is the only thing here that
    /// knows which one made it. `res`/`params` are recorded so a caller can still
    /// ask [`Self::matches`], though a per-frame grid is replaced before it can go
    /// stale.
    pub fn adopt(cells: Vec<[f32; 2]>, res: [f32; 2], params: [f32; 4]) -> Option<Map> {
        (cells.len() == EDGE * EDGE).then_some(Map { res, params, cells })
    }

    /// Whether this map still describes `res`/`params`. The holder rebakes when
    /// not; nothing else invalidates it, because nothing else is an input.
    pub fn matches(&self, res: [f32; 2], params: [f32; 4]) -> bool {
        self.res == res && self.params == params
    }

    /// Bilinear lookup. Inputs outside the unit square clamp — the warp's own
    /// domain is the output, and a pointer cannot be outside it.
    pub fn sample(&self, u: f64, v: f64) -> (f64, f64) {
        sample_cells(&self.cells, u, v)
    }
}

/// Sample a grid held by someone else.
///
/// The GPU producer publishes an `Arc<Vec<_>>` that the pointer reads on every
/// motion event; wrapping it in a `Map` to sample it meant COPYING 128 KB per
/// event — a quarter of a gigabyte a second at 1 kHz polling, to answer a
/// question about one point.
pub fn sample_cells(cells: &[[f32; 2]], u: f64, v: f64) -> (f64, f64) {
    {
        let last = (EDGE - 1) as f64;
        let (fx, fy) = ((u.clamp(0.0, 1.0) * last), (v.clamp(0.0, 1.0) * last));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(EDGE - 1), (y0 + 1).min(EDGE - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let at = |x: usize, y: usize| cells[y * EDGE + x];
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let lerp = |p: f32, q: f32, t: f64| p as f64 + (q as f64 - p as f64) * t;
        (
            lerp(lerp(a[0], b[0], tx) as f32, lerp(c[0], d[0], tx) as f32, ty),
            lerp(lerp(a[1], b[1], tx) as f32, lerp(c[1], d[1], tx) as f32, ty),
        )
    }
}
