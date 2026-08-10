// TB STRESS 22 — "tb-rects-offload". The first bundle that uses WINDOW DATA and
// still runs on the worker.
//
// One before-content pass declaring only `window-rects`, so the whole graph is
// `Offload::Whole`. Window GEOMETRY is plain numbers: the compositor publishes it
// to a shared slot each frame and the worker reads it, with no buffer import, no
// acquire fence and nothing to keep alive across devices. Window TEXTURES are
// client buffers and do not cross yet, which is exactly the line this bundle
// sits on.
//
// It paints a soft halo into the BACKGROUND beneath each window, plus a bright
// footprint outline. Because it is a background pass the windows composite over
// it normally — you should see the glow spill out from behind every window edge.
//
// What to look for, with triple buffering on:
//   * `background worker: pane running graph offthread` in the log — it offloaded;
//   * the halos TRACK the windows as you drag them. Any lag is the shared slot
//     being one frame behind, which is the documented clock divergence between
//     bands rather than a fault. Watch how much.
//   * no halos at all, but a background -> the slot is empty: either no client
//     windows, or the compositor is not publishing.
//
// @prop reach float default=0.05 min=0.0 max=0.30 step=0.005 label="Halo reach" group="Rects offload"
// @prop lift  float default=1.0  min=0.0 max=3.0  step=0.05  label="Halo strength" group="Rects offload"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

// `_pad: vec3<u32>` is 16-byte aligned, so the arrays start at offset 32.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;

fn sd_box(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let d = abs(p) - half;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let reach = pc.params[0].x;
    let lift = pc.params[0].y;

    // Moving backdrop, so a frozen worker is obvious too.
    let band = 0.5 + 0.5 * sin((uv.x - uv.y) * 10.0 + t * 0.4);
    var col = mix(vec3<f32>(0.03, 0.04, 0.09), vec3<f32>(0.08, 0.05, 0.16), band);

    var glow = 0.0;
    var edge = 0.0;
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let d = sd_box(uv - (r.xy + r.zw * 0.5), r.zw * 0.5);
        glow = glow + smoothstep(reach, 0.0, max(d, 0.0));
        edge = max(edge, smoothstep(0.0035, 0.0, abs(d)));
    }
    col = col + vec3<f32>(0.25, 0.65, 1.0) * min(glow, 2.0) * lift;
    col = mix(col, vec3<f32>(1.0, 0.95, 0.6), edge * 0.9);
    return vec4<f32>(col, 1.0);
}
