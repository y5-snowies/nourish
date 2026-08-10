// TB STRESS 11 — "tb-window-mirror", pass 1/2 (before-content). A strong teal
// gradient, deliberately unlike the stock parallax so a silent fallback to the
// single-pass path is obvious at a glance.
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
    let c = mix(vec3<f32>(0.02, 0.16, 0.18), vec3<f32>(0.03, 0.05, 0.12), uv.y);
    return vec4<f32>(c, 1.0);
}
