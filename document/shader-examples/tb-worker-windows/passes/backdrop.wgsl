// Head band, inline. A flat two-tone wash: the point of this bundle is where the
// tail pass finds WINDOWS, so the background must not be busy enough to hide it.
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
    let uv = frag.xy / res;
    return vec4<f32>(mix(vec3<f32>(0.05, 0.07, 0.11), vec3<f32>(0.09, 0.12, 0.18), uv.y), 1.0);
}
