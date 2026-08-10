// TB STRESS 2/10 — "tb-chain-6", reused downsample blit. `res_zoom_time.xy` is
// the CURRENT target's size, so sampling by normalised uv rescales correctly.
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
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    return vec4<f32>(textureSample(src, samp, uv).rgb, 1.0);
}
