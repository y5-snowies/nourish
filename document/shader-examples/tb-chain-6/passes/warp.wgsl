// TB STRESS 2/10 — "tb-chain-6", reused warp (9 taps). Cheap per pass; the point
// is the NUMBER of passes, not the cost of any one.
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
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let off = vec2<f32>(sin(uv.y * 8.0 + t) , cos(uv.x * 8.0 - t)) / res * 3.0;
    var acc = vec3<f32>(0.0);
    for (var i = -1; i <= 1; i = i + 1) {
        for (var j = -1; j <= 1; j = j + 1) {
            let d = vec2<f32>(f32(i), f32(j)) / res;
            acc = acc + textureSample(src, samp, uv + off + d).rgb;
        }
    }
    return vec4<f32>(acc / 9.0, 1.0);
}
