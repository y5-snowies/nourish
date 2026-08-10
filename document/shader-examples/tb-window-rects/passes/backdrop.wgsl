// TB STRESS 6/10 — "tb-window-rects", pass 1/2 (before-content).
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    return vec4<f32>(mix(vec3<f32>(0.04, 0.06, 0.10), vec3<f32>(0.10, 0.14, 0.20), uv.y), 1.0);
}
