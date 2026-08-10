// TB STRESS — "tb-crt-spin". A CRT CURVE OVER ROTATING WINDOWS, AND A POINTER
// THAT KNOWS ABOUT IT.
//
// Two passes, deliberately:
//
//   1. `world` — the shared `tb-window-owned` pass, `windows: "world"`, which
//      composites background + windows (each on its own sway) + iced-world
//      panels into an intermediate `band`;
//   2. `crt` (this) — barrel-warps that band into `output`.
//
// One bundle would have been shorter. Two exists because the pointer warp is a
// CHAIN, applied pass by pass, and a single-pass bundle cannot show that: with
// two, the chain has one entry today (this pass) and the window rotation is
// visibly the second entry to add later, rather than a rewrite.
//
// WHAT THIS BUNDLE IS ACTUALLY TESTING
// ------------------------------------
// Not the curve — a barrel distortion is four lines. It is that
// `lib/warp.wgsl::hit_inverse` is the ONLY description of the displacement, and
// that both readers of it agree:
//
//   * THIS PASS samples through it, so the curve you see is that function;
//   * THE ENGINE parses the same file and interprets it on the CPU
//     (`shader.hit`), so the cursor is corrected by that same function.
//
// There is no Rust twin of the curve to fall out of step with. Edit the maths in
// `lib/warp.wgsl` — bend it harder, make it a pincushion, replace it with a
// ripple — and the pointer follows with nothing else touched. That property is
// the whole point; the CRT look is just a legible way to see it.
//
// HOW TO TELL IT WORKS
// --------------------
// Curvature is strongest at the corners, so test there, not in the middle where
// every warp is the identity.
//
// | What you do | Working | Broken |
// |---|---|---|
// | hover a window edge near a corner | highlight follows the BENT edge | highlights on the straight edge, offset outward |
// | click a corner window | that window focuses | the one behind/beside it focuses |
// | drag a window to a corner | it tracks the cursor the whole way | it drifts away as curvature grows |
// | set `curve` to 0 | pointer is pixel-exact everywhere | still offset ⇒ warp is not identity at k=0 |
// | hover the settings panel | normal | offset ⇒ the warp is wrongly applied to the screen band |
//
// That last row matters as much as the others. Screen-space content — iced
// screen UI, layer-shell top/overlay, and the cursor sprite itself — is drawn
// AFTER the after-content stage (`split_at`), so it was never displaced and must
// be hit at the TRUE cursor position. The warp applies to the band this bundle
// composited, and to nothing else.
//
// KNOWN GAP, ON PURPOSE
// ---------------------
// The window SWAY is not in the pointer map yet. Windows are hit at their
// unrotated rects, so a strongly-swayed window is clickable slightly off its
// drawn position. The CRT curve is corrected; the rotation is not. It folds into
// the same `hit_inverse` later as another branch — pointwise evaluation handles
// the discontinuity at a window edge natively, which is exactly why the map is a
// function and not a sampled grid.
//
// @prop curve   float default=0.18 min=0.0 max=0.6  step=0.01 label="Curvature"   group="CRT"
// @prop scan    float default=0.35 min=0.0 max=1.0  step=0.01 label="Scanlines"   group="CRT"
// @prop vignette float default=0.5 min=0.0 max=1.5  step=0.01 label="Vignette"    group="CRT"

#import crt_both::warp::hit_inverse

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let curve = pc.params[0].x;
    let scan = pc.params[0].y;
    let vig = pc.params[0].z;

    // THE one displacement. `params.x` is `curve`, matching what the engine feeds
    // the interpreter — the first four @props, in order.
    let warped = hit_inverse(uv, res, vec4<f32>(curve, 0.0, 0.0, 0.0));

    // Off-screen after bending: the tube's black surround. Sampling clamped here
    // would smear the edge row outward and, worse, make the region LOOK like
    // content while the pointer map says it came from outside — so it is a hard
    // cutoff in both.
    if (warped.x < 0.0 || warped.x > 1.0 || warped.y < 0.0 || warped.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var col = textureSampleLevel(src, samp, warped, 0.0).rgb;

    // Scanlines in WARPED space, so they bend with the tube instead of sitting
    // flat on top of it — a straight scanline over a curved image is the giveaway
    // that the curve is a post-effect rather than the geometry.
    let line = 0.5 + 0.5 * cos(warped.y * res.y * 3.14159);
    col = col * (1.0 - scan * 0.35 * line);

    // Corner falloff, radial in the same aspect-corrected space the warp uses.
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (warped - vec2<f32>(0.5, 0.5)) * a;
    col = col * (1.0 - vig * 0.6 * dot(c, c));

    return vec4<f32>(col, 1.0);
}
