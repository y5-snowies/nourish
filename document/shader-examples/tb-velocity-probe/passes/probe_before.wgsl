// TB STRESS 8/10 — "tb-velocity-probe", pass 1/2 (BEFORE-content).
//
// DIAGNOSTIC. Draws the engine push state this band actually received:
//   • TOP bar    (cyan)   — decoded |pan velocity|, log-scaled
//   • SECOND bar (yellow) — fract(time), a 1 Hz sawtooth
//
// The AFTER pass draws the same two bars lower down from ITS OWN push. On the
// inline path the pairs are identical. Once a band runs on a paced worker the
// bars SEPARATE — and that separation is the clock divergence that makes any
// effect spanning both bands (velocity- or time-driven) inconsistent.
//
// This is also the ground-truth test for the velocity lane itself: pan the world
// and the cyan bar must grow. If it stays flat, the lane is not arriving and any
// velocity-driven bundle (motion-blur) is silently dead.
//
// Lane ABI: two snorm16 halves scaled by VELOCITY_LANE_SCALE (16384).

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;
    let speed = length(vel);
    // log scale: 1 px/s -> ~0, 3000 px/s -> ~1
    let norm = clamp(log2(1.0 + speed) / 11.5, 0.0, 1.0);

    var col = mix(vec3<f32>(0.04, 0.05, 0.09), vec3<f32>(0.08, 0.10, 0.16), uv.y);

    // Row 0: velocity magnitude (cyan).
    if (uv.y > 0.02 && uv.y < 0.06 && uv.x < norm) {
        col = vec3<f32>(0.1, 0.9, 1.0);
    }
    // Row 1: fract(time) (yellow).
    if (uv.y > 0.08 && uv.y < 0.12 && uv.x < fract(t)) {
        col = vec3<f32>(1.0, 0.85, 0.2);
    }
    return vec4<f32>(col, 1.0);
}
