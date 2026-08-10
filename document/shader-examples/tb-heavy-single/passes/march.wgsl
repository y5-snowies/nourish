// TB STRESS 1/10 — "tb-heavy-single".
// Shape: ONE before-content pass, no inputs, no `needs`, writes `output`.
// Offload class: WHOLE (fully worker-eligible — nothing crosses a boundary).
//
// What it stresses: raw GPU cost in a single pass. This is the bundle triple
// buffering should help MOST — all the work is background work, and none of it
// needs anything the compositor owns. If TB does not lift the frame rate here,
// the worker path is not being taken at all.
//
// Watch for: input latency while panning. Inline, this cost lands in the
// compositor's single command buffer on the thread that dispatches input.
//
// @prop steps float default=48.0 min=8.0  max=96.0 step=1.0  label="March steps" group="TB stress"
// @prop scale float default=1.6  min=0.5  max=4.0  step=0.05 label="Field scale" group="TB stress"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

fn field(p: vec3<f32>) -> f32 {
    var q = p;
    var d = 0.0;
    d = d + sin(q.x * 1.3 + q.z * 0.7) * 0.35;
    d = d + sin(q.y * 1.7 - q.z * 1.1) * 0.30;
    d = d + sin(q.z * 0.9 + q.x * 1.5) * 0.25;
    return length(q) - (1.6 + d);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let steps = i32(clamp(pc.params[0].x, 8.0, 96.0));
    let fscale = pc.params[0].y;

    let uv = (frag.xy / max(res, vec2<f32>(1.0))) * 2.0 - vec2<f32>(1.0);
    let aspect = res.x / max(res.y, 1.0);
    let dir = normalize(vec3<f32>(uv.x * aspect, -uv.y, 1.4));
    var ro = vec3<f32>(sin(t * 0.15) * 0.6, cos(t * 0.11) * 0.4, -4.0);

    var acc = 0.0;
    var tt = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        let p = (ro + dir * tt) * fscale;
        let d = field(p);
        acc = acc + exp(-abs(d) * 2.2) * 0.06;
        tt = tt + max(abs(d) * 0.55, 0.03);
    }

    let base = vec3<f32>(0.04, 0.05, 0.10);
    let glow = vec3<f32>(0.35, 0.55, 0.95) * acc;
    return vec4<f32>(base + glow, 1.0);
}
