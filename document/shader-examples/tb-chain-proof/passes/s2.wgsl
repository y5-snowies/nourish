// TB STRESS 21 — "tb-chain-proof", stage 2 of 3. Reads the half-res target back
// at full resolution, copies it forward, and adds signature 2 (pure blue).
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var prev: texture_2d<f32>;

fn in_strip(uv: vec2<f32>, k: f32) -> bool {
    let lo = 0.10 + k * 0.08;
    return uv.y >= lo && uv.y <= lo + 0.05;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    if (in_strip(uv, 2.0)) {
        return vec4<f32>(0.0, 0.0, 1.0, 1.0);   // signature 2
    }
    return vec4<f32>(textureSample(prev, samp, uv).rgb, 1.0);
}
