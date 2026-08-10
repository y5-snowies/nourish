// TB STRESS 6/10 — "tb-window-rects", pass 2/2 (AFTER-CONTENT, needs window-rects).
//
// Draws a crisp 2px outline on every client window rect. `window-rects` is the
// CHEAP engine interface: it is just numbers in a UBO, so it is the one piece of
// window data that could cross to a worker with no dmabuf export and no fence —
// contrast `tb-window-tex`, which needs the bindless texture array.
//
// Diagnostic value: the outline must track windows EXACTLY as you drag them. Any
// lag between the outline and the window is the clock/frame gap between the band
// that produced the rects and the band that drew them. Under a paced worker that
// gap becomes visible here first.
//
// @prop thick float default=2.0 min=1.0 max=8.0 step=0.5 label="Outline px" group="TB stress"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

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
    let uv = frag.xy / res;
    let thick = pc.params[0].x / max(res.y, 1.0);
    var col = textureSample(scene, samp, uv).rgb;

    var edge = 0.0;
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let d = sd_box(uv - (r.xy + r.zw * 0.5), r.zw * 0.5);
        edge = max(edge, smoothstep(thick, 0.0, abs(d)));
    }
    // Count readout: a small bar per window along the top edge.
    let slot = floor(uv.x * 64.0);
    let bar = select(0.0, 1.0, uv.y < 0.004 && slot < f32(n));
    col = mix(col, vec3<f32>(0.2, 1.0, 0.5), max(edge, bar));
    return vec4<f32>(col, 1.0);
}
