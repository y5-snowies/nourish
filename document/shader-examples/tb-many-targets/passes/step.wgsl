// TB STRESS 9/10 — "tb-many-targets", reused chain step. Normalised-uv sampling
// so it is correct at every scale change in the chain.
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
    let d = 1.0 / res;
    var acc = textureSample(src, samp, uv).rgb * 4.0;
    acc = acc + textureSample(src, samp, uv + vec2<f32>( d.x, 0.0)).rgb;
    acc = acc + textureSample(src, samp, uv + vec2<f32>(-d.x, 0.0)).rgb;
    acc = acc + textureSample(src, samp, uv + vec2<f32>(0.0,  d.y)).rgb;
    acc = acc + textureSample(src, samp, uv + vec2<f32>(0.0, -d.y)).rgb;
    return vec4<f32>(acc / 8.0, 1.0);
}
