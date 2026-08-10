// TB STRESS 5/10 — "tb-after-history", pass 1/2 (before-content).
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
    // A moving vertical bar: gives `history` something unambiguous to lag behind.
    let x = fract(t * 0.12);
    let bar = smoothstep(0.02, 0.0, abs(uv.x - x));
    let base = mix(vec3<f32>(0.05, 0.06, 0.11), vec3<f32>(0.10, 0.09, 0.18), uv.y);
    return vec4<f32>(base + vec3<f32>(0.55, 0.40, 0.15) * bar, 1.0);
}
