// TB STRESS 3/10 — "tb-wide-32f", pass 2/3: a WIDE 25-tap read of a full-res
// rgba32f target. Deliberately bandwidth-bound rather than ALU-bound — this is
// the shape that saturates memory on integrated GPUs (vc4/v3d) while barely
// registering on a discrete card.
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
    var acc = vec3<f32>(0.0);
    for (var i = -2; i <= 2; i = i + 1) {
        for (var j = -2; j <= 2; j = j + 1) {
            let d = vec2<f32>(f32(i), f32(j)) * 3.0 / res;
            acc = acc + textureSample(src, samp, uv + d).rgb;
        }
    }
    return vec4<f32>(acc / 25.0, 1.0);
}
