// TB STRESS 4/10 — "tb-after-trivial", pass 2/2 (AFTER-CONTENT).
// Deliberately the CHEAPEST possible after pass: one sample, one multiply.
// Its whole purpose is to measure the FIXED cost of the after-content path —
// the offscreen `content` image, the extra render pass, and the forced
// full-frame redraw (damage scissoring is disabled whenever the offscreen path
// is active, because after passes sample neighbouring pixels).
//
// Compare frame rate against `tb-after-none`-style bundles: any difference here
// is pure overhead, not shader cost. This is the number that decides whether
// after-content is worth offloading at all.
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
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let c = textureSample(scene, samp, uv).rgb;
    return vec4<f32>(c * vec3<f32>(1.02, 1.0, 0.98), 1.0);
}
