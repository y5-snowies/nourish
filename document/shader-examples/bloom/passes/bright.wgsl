// bloom pass 2/5 — "bright": extract pixels above a luminance threshold from
// `scene` into the half-res `bright` target. Samples one input (`src` = scene)
// and imports the shared luminance helper — the naga_oil `#import` proof.
//
// Input binding ABI: sampler at @group(0) @binding(0); each `inputs` entry binds
// at @binding(1+i) in SORTED-KEY order. Here the only input is `src`.
//
// @prop threshold float default=0.45 min=0.0 max=3.0 step=0.01 label="Bloom threshold" group="Bloom"

#import color::luminance

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>, // @prop slot 0 = threshold
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / pc.res_zoom_time.xy;
    let threshold = pc.params[0].x; // @prop threshold
    let c = textureSample(src, samp, uv).rgb;
    let lum = luminance(c);
    // Soft knee: keep only the over-threshold energy, preserving hue.
    let keep = max(lum - threshold, 0.0);
    let scaled = c * (keep / max(lum, 1e-4));
    return vec4<f32>(scaled, 1.0);
}
