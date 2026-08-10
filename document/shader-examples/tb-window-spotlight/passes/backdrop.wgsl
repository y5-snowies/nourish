// TB STRESS 12 — "tb-window-spotlight", pass 1/2 (before-content).
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let band = 0.5 + 0.5 * sin(uv.y * 30.0 + t * 0.8);
    return vec4<f32>(vec3<f32>(0.05, 0.04, 0.10) + vec3<f32>(0.10, 0.04, 0.16) * band, 1.0);
}
