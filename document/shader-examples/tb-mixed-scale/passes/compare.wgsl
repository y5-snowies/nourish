// TB STRESS 10/10 — "tb-mixed-scale", pass 3/3: side-by-side.
// LEFT half = the full-res grid. RIGHT half = the quarter-res round trip,
// upsampled. A vertical seam at x=0.5 divides them.
//
// What to look for: the right side should be a soft blur of the left, CENTRED
// identically. If the right side is SHIFTED relative to the left, a half-texel
// offset has crept into the scaled-target sampling — the classic bug when
// `res_zoom_time.xy` is taken as the OUTPUT size rather than the CURRENT
// target's size. Under triple buffering the worker allocates its own targets, so
// this is where a scale mismatch between compositor and worker would surface.
//
// Sorted input names: hi->1, lo->2.
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var hi: texture_2d<f32>;
@group(0) @binding(2) var lo: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    var col = textureSample(hi, samp, uv).rgb;
    if (uv.x > 0.5) {
        col = textureSample(lo, samp, uv).rgb;
    }
    // Seam marker.
    if (abs(uv.x - 0.5) < 0.0012) {
        col = vec3<f32>(1.0, 0.3, 0.3);
    }
    return vec4<f32>(col, 1.0);
}
