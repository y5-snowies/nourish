// bloom pass 5/5 — "combine": add the blurred bloom back onto the original
// scene and tone-map, writing the swapchain (`output`). Two inputs; recall
// bindings are assigned in SORTED-KEY order, so `bloom` → binding 1, `scene` →
// binding 2. Imports the shared tone-map helper.
//
// @prop intensity float default=1.80 min=0.0 max=4.0 step=0.01 label="Bloom intensity" group="Bloom"
// @prop exposure  float default=1.10 min=0.1 max=3.0 step=0.01 label="Exposure"        group="Bloom"

#import color::tonemap

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>, // @prop slots: 0 = intensity, 1 = exposure
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / pc.res_zoom_time.xy;
    let intensity = pc.params[0].x; // @prop intensity
    let exposure = pc.params[0].y;  // @prop exposure
    let base = textureSample(scene, samp, uv).rgb;
    let glow = textureSample(bloom, samp, uv).rgb * intensity;
    let mapped = tonemap(base + glow, exposure);
    return vec4<f32>(mapped, 1.0);
}
