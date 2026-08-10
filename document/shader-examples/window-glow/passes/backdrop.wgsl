// window-glow pass 1/2 — "backdrop" (before-content): a dark, calm background so
// the glow halos around windows read clearly. Fragment-only.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / pc.res_zoom_time.xy;
    let col = mix(vec3<f32>(0.02, 0.03, 0.06), vec3<f32>(0.04, 0.05, 0.10), uv.y);
    return vec4<f32>(col, 1.0);
}
