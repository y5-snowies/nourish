// TB STRESS 3/10 — "tb-wide-32f", pass 1/3.
// Offload class: WHOLE. Two FULL-RES rgba32f intermediates — 16 bytes/px each.
// At 4K that is ~132 MB of intermediates per pane, and the worker allocates its
// own set PER SLOT. With slots=3 that is the allocation cliff to watch: refused
// allocations gate the pane (see `publish.attempt`) and the background stalls.
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
    let a = sin(uv.x * 20.0 + t) * cos(uv.y * 18.0 - t * 0.6);
    return vec4<f32>(0.5 + 0.5 * a, 0.35 + 0.3 * a, 0.65 - 0.25 * a, 1.0);
}
