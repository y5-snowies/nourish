// TB STRESS 2/10 — "tb-chain-6", pass 6/6. Sorted input names: base->1, small->2.
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var base: texture_2d<f32>;
@group(0) @binding(2) var small: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let a = textureSample(base, samp, uv).rgb;
    let b = textureSample(small, samp, uv).rgb;
    return vec4<f32>(a * 0.55 + b * 0.85, 1.0);
}
