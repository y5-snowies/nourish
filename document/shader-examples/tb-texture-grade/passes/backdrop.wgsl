// TB — `tb-texture-grade`, pass 1/2. A plain gradient, so there is something
// behind the windows for the grade to act on. Nothing here is about textures.

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
    let t = pc.res_zoom_time.w;
    let band = 0.5 + 0.5 * sin(uv.x * 3.0 + uv.y * 1.5 + t * 0.08);
    let c = mix(vec3<f32>(0.10, 0.11, 0.13), vec3<f32>(0.20, 0.19, 0.17), band);
    return vec4<f32>(c, 1.0);
}
