// TB STRESS 9/10 — "tb-many-targets", pass 1/9.
// EIGHT intermediate targets across FOUR scales and THREE formats, all
// before-content (offload class WHOLE). This is the allocation-churn bundle:
// every target is reallocated on output resize, and under triple buffering the
// worker holds its own set PER SLOT. Resize the output / hotplug a monitor while
// this runs — a refused allocation gates the pane (see `publish.attempt`, which
// retries on CHANGE rather than on a timer), so a leak or a missed retry shows
// up as a background that never comes back.
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
    let a = 0.5 + 0.5 * sin((uv.x + uv.y) * 12.0 + t);
    return vec4<f32>(a * 0.6, 0.25 + a * 0.3, 0.7 - a * 0.3, 1.0);
}
