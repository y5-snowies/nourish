// tb-crt pass 1/2 — a bright test-card backdrop so the CRT pass has saturated
// colour and hard edges to work on (both show tube artefacts clearly).
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
    // Vertical colour bars, SMPTE-ish, plus a dark lower band.
    let bar = u32(floor(uv.x * 7.0));
    var c = vec3<f32>(0.75);
    if (bar == 1u) { c = vec3<f32>(0.75, 0.75, 0.0); }
    if (bar == 2u) { c = vec3<f32>(0.0, 0.75, 0.75); }
    if (bar == 3u) { c = vec3<f32>(0.0, 0.75, 0.0); }
    if (bar == 4u) { c = vec3<f32>(0.75, 0.0, 0.75); }
    if (bar == 5u) { c = vec3<f32>(0.75, 0.0, 0.0); }
    if (bar == 6u) { c = vec3<f32>(0.0, 0.0, 0.75); }
    if (uv.y > 0.75) { c = c * 0.18; }
    return vec4<f32>(c, 1.0);
}
