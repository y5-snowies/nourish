// The tube, applied AFTER content — shared by `tb-crt-after-pointwise` and
// `tb-crt-after-live`, which differ in `hit.evaluate` and in nothing else.
//
// WHAT THIS PAIR IS FOR
// ---------------------
// Every other warping example realises its displacement in a BEFORE-content pass
// over a band it composited itself (`windows: "world"`). `mp-crt` is the only one
// that warps the composited desktop from an after-content pass — and it is
// `map_static`, so `pointwise` and `map` had no after-content coverage at all.
//
// That gap is not cosmetic. Which band a warp lives in decides whether the engine's
// own world-band elements go through it, and the canvas cursor box is one of them:
// it is the LAST element in the content band, which smithay draws first, so it
// precedes every window in the op list and `split_at` always leaves it inside
// `content`. Under this bundle the box is therefore warped BY the shader and must
// be positioned at the warp-corrected point; under `crt-input-map` nothing warps it
// and it must be positioned at the hand. Two branches, and until this pair existed
// only one of them was reachable with a pointwise or live-map warp.
//
// HOW TO TELL IT WORKS
// --------------------
// Corners, not the middle — every warp is the identity at the centre.
//
// | What you do | Working | Broken |
// |---|---|---|
// | put the pointer in a corner | the box sits under the hardware cursor | box offset outward, roughly twice the bend |
// | set `curve` to 0 | box and sprite pixel-exact | still offset ⇒ a correction is being applied that should not be |
// | hover a window edge near a corner | highlight follows the BENT edge | highlight on the straight edge |
// | hover the settings panel | normal | offset ⇒ the warp is reaching the screen band |
//
// The first row is the one this pair adds. Compare against `crt-input-map`, which
// takes the other branch: both must land the box under the cursor, by opposite
// routes.
//
// `curve` FIRST: this pass's push is packed in @prop order, so `params[0].x` is
// what the engine hands the interpreter as `params.x`.
//
// @prop curve    float default=0.18 min=0.0 max=0.6 step=0.01 label="Curvature" group="Tube"
// @prop scan     float default=0.35 min=0.0 max=1.0 step=0.01 label="Scanlines" group="Tube"
// @prop vignette float default=0.5  min=0.0 max=1.5 step=0.01 label="Vignette"  group="Tube"

#import crt_map::warp::hit_inverse

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let curve = pc.params[0].x;
    let scan = pc.params[0].y;
    let vig = pc.params[0].z;

    // THE one displacement, from `crt-input-map/lib/warp.wgsl` — the same file the
    // engine parses and interprets on the CPU. No Rust twin to drift from.
    let warped = hit_inverse(uv, res, vec4<f32>(curve, 0.0, 0.0, 0.0));

    // Off the tube after bending: hard black, not a clamped smear. A clamped edge
    // would LOOK like content while the pointer map says it came from outside.
    if (warped.x < 0.0 || warped.x > 1.0 || warped.y < 0.0 || warped.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var col = textureSampleLevel(scene, samp, warped, 0.0).rgb;

    // Scanlines in WARPED space so they bend with the tube. Straight scanlines over
    // a curved image is the giveaway that the curve is a post-effect.
    let line = 0.5 + 0.5 * cos(warped.y * res.y * 3.14159);
    col = col * (1.0 - scan * 0.35 * line);

    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (warped - vec2<f32>(0.5, 0.5)) * a;
    col = col * (1.0 - vig * 0.6 * dot(c, c));

    return vec4<f32>(col, 1.0);
}
