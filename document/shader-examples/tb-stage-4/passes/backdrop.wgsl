// STAGE 4 VALIDATOR, pass 1/2 — the HEAD band, which stays INLINE.
//
// Stage 4 inverts every other stage: the compositor composites background and
// windows, and the worker decorates the result. So this pass runs on the
// compositor, and the `decorate` pass runs off-thread.
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
    let bar = step(0.5, fract(uv.x * 8.0 + t * 0.05));
    return vec4<f32>(mix(vec3<f32>(0.05, 0.08, 0.16), vec3<f32>(0.13, 0.06, 0.20), bar), 1.0);
}
