// TB STRESS 20 — "tb-cadence", pass 1/2. Runs at `"cadence": 8` — every EIGHTH
// frame. Between runs it is simply not recorded, and its target keeps the
// previous result: the intermediates are allocated once, not per frame, so
// "hold" costs no extra storage and no copy.
//
// It draws a sweeping arm from `time`, so the held frames are obvious: the arm
// jumps in eight-frame steps while the companion arm in `out.wgsl` sweeps
// smoothly. That side-by-side IS the test.
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
    let p = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    // A sweeping arm: angle from time, thin wedge.
    let ang = atan2(p.y, p.x);
    let sweep = t * 1.2;
    let d = abs(fract((ang - sweep) / 6.2831853 + 0.5) - 0.5);
    let arm = smoothstep(0.02, 0.0, d) * smoothstep(0.42, 0.10, length(p));
    return vec4<f32>(vec3<f32>(1.0, 0.45, 0.15) * arm, 1.0);
}
