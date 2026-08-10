// Head band, inline (stage 4 takes the tail). A slow-moving diagonal so the
// trail below has something with a known velocity to smear.
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
    let band = fract((uv.x + uv.y) * 3.0 - t * 0.35);
    let c = smoothstep(0.85, 1.0, band);
    return vec4<f32>(mix(vec3<f32>(0.03, 0.05, 0.10), vec3<f32>(0.35, 0.55, 0.85), c), 1.0);
}
