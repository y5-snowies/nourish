// TB — the chain, the workhorse: ONE file, SIX passes.
//
// This is the shader-def lesson. A separable Gaussian is two passes that differ
// only in which axis they walk, and the manifest compiles this same source twice
// per level — once with `"defines": { "HORIZONTAL": "true" }` and once without:
//
//     { "shader": "passes/blur.wgsl", "output": "l1_b",
//       "defines": { "HORIZONTAL": "true" } }
//     { "shader": "passes/blur.wgsl", "output": "l1_a" }
//
// Six of this bundle's ten passes are this file. The alternative is two files
// that must be kept identical apart from one vector, which is the arrangement
// that eventually gets one of them edited and not the other.
//
// PING-PONG. Each level owns two targets, `_a` and `_b`, and the pair alternates:
// H reads `_a` writes `_b`, V reads `_b` writes `_a`. A pass cannot read the
// image it is writing, so the second target is not an optimisation — it is the
// only way to chain two passes over the same data.
//
// @prop radius float default=1.0 min=0.25 max=3.0 step=0.05 label="Blur radius" group="Bleed"

#ifdef HORIZONTAL
const AXIS: vec2<f32> = vec2<f32>(1.0, 0.0);
#else
const AXIS: vec2<f32> = vec2<f32>(0.0, 1.0);
#endif

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
    let radius = max(pc.params[0].x, 0.05);
    // A texel of THIS level — so the same radius means the same number of texels
    // at every scale, and the coarser levels blur proportionally wider in screen
    // terms. That is the whole point of a pyramid: one kernel, many extents.
    let step = AXIS * radius / res;

    // 9 taps, Gaussian-ish weights, normalised.
    var acc = textureSampleLevel(src, samp, uv, 0.0).rgb * 0.227;
    var w = 0.227;
    let k = array<f32, 4>(0.194, 0.121, 0.054, 0.016);
    for (var i = 1; i <= 4; i = i + 1) {
        let d = step * f32(i);
        let kw = k[i - 1];
        acc = acc + textureSampleLevel(src, samp, uv + d, 0.0).rgb * kw;
        acc = acc + textureSampleLevel(src, samp, uv - d, 0.0).rgb * kw;
        w = w + kw * 2.0;
    }
    return vec4<f32>(acc / w, 1.0);
}
