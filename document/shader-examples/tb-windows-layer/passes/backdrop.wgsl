// tb-windows-layer pass 1/2 — a strong checkerboard, so anything the window layer
// contributes is unmistakable against it.
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
    let c = step(0.5, fract(uv.x * 12.0)) + step(0.5, fract(uv.y * 8.0));
    let k = fract(c * 0.5) * 2.0;
    return vec4<f32>(mix(vec3<f32>(0.05, 0.07, 0.13), vec3<f32>(0.11, 0.15, 0.24), k), 1.0);
}
