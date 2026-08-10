// TB STRESS 2/10 — "tb-chain-6", pass 1/6. Before-content seed, no inputs.
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / max(res, vec2<f32>(1.0));
    let r = 0.5 + 0.5 * sin(uv.x * 6.0 + t * 0.7);
    let g = 0.5 + 0.5 * sin(uv.y * 5.0 - t * 0.5);
    let b = 0.5 + 0.5 * sin((uv.x + uv.y) * 4.0 + t * 0.3);
    return vec4<f32>(vec3<f32>(r, g, b) * 0.35, 1.0);
}
