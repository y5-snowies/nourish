// tb-window-frost pass 1/2 — a busy wallpaper, so frosting is obvious.
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
    let s = sin(uv.x * 14.0 + t * 0.3) * cos(uv.y * 11.0 - t * 0.2);
    var col = mix(vec3<f32>(0.04, 0.10, 0.18), vec3<f32>(0.20, 0.06, 0.16), 0.5 + 0.5 * s);
    col = col + vec3<f32>(0.10, 0.16, 0.20) * step(0.92, fract((uv.x + uv.y) * 9.0));
    return vec4<f32>(col, 1.0);
}
