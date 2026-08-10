// TB STRESS 8/10 — "tb-velocity-probe", pass 2/2 (AFTER-content).
// Same two readouts as the before pass, drawn LOWER. Compare each pair:
// aligned = one clock; separated = the bands are running on different frames.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;
    let speed = length(vel);
    let norm = clamp(log2(1.0 + speed) / 11.5, 0.0, 1.0);

    var col = textureSample(scene, samp, uv).rgb;

    // Row 2: velocity magnitude as seen by the AFTER band (dim cyan).
    if (uv.y > 0.14 && uv.y < 0.18 && uv.x < norm) {
        col = vec3<f32>(0.0, 0.55, 0.7);
    }
    // Row 3: fract(time) as seen by the AFTER band (dim yellow).
    if (uv.y > 0.20 && uv.y < 0.24 && uv.x < fract(t)) {
        col = vec3<f32>(0.65, 0.5, 0.1);
    }
    return vec4<f32>(col, 1.0);
}
