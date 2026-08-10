// TB STRESS 4/10 — "tb-after-trivial", pass 1/2 (before-content).
// Offload class: BEFORE-BAND. This pass alone is worker-eligible; the `tint`
// pass is not, so the graph cannot be offloaded whole.
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
    let c = mix(vec3<f32>(0.06, 0.08, 0.14), vec3<f32>(0.16, 0.10, 0.22), uv.y);
    return vec4<f32>(c, 1.0);
}
