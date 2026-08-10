// bloom pass 3+4/5 — "blur": a separable 9-tap Gaussian. The SAME source is
// compiled twice: `blurH` sets the `HORIZONTAL` shader-def (→ horizontal taps
// into blur_a), `blurV` leaves it unset (→ vertical taps into blur_b). This is
// the naga_oil shader-def variant proof — one file, two pipelines.
//
// Ping-pong: blurH reads `bright`, blurV reads blurH's output `blur_a`.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

const W = array<f32, 5>(0.227027, 0.194595, 0.121622, 0.054054, 0.016216);

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let uv = frag.xy / res;
#ifdef HORIZONTAL
    let step_uv = vec2<f32>(1.0 / res.x, 0.0);
#else
    let step_uv = vec2<f32>(0.0, 1.0 / res.y);
#endif
    var sum = textureSample(src, samp, uv).rgb * W[0];
    for (var i = 1; i < 5; i = i + 1) {
        let off = step_uv * f32(i);
        sum = sum + textureSample(src, samp, uv + off).rgb * W[i];
        sum = sum + textureSample(src, samp, uv - off).rgb * W[i];
    }
    return vec4<f32>(sum, 1.0);
}
