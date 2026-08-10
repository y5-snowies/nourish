// TB STRESS 23 — "tb-windows-layer". Proves the `windows` built-in target.
//
// `windows` is the composited WINDOW LAYER: every client window drawn over
// TRANSPARENCY, with the background excluded. It was reserved in the ABI from the
// start and never implemented — a manifest naming it used to fail to load and
// fall back silently. Now it is produced, consumption-gated like `history`, so a
// bundle that never mentions it costs nothing.
//
// Why it matters beyond the effect: it is how a pass reaches window CONTENT
// without touching client buffers. By the time windows are drawn into this layer
// the compositor has already resolved dmabuf AND SHM surfaces into ordinary
// images, so a shader reads both the same way — and, when this eventually crosses
// to the worker, nothing client-owned has to cross a device boundary or carry an
// acquire fence with it.
//
// The pass splits the screen down the middle:
//   LEFT  — the ordinary composited scene (`content`).
//   RIGHT — the window layer alone, over a red field.
//
// | What you see on the right | Meaning |
// |---|---|
// | windows on red, background absent | correct: the layer is window-only |
// | the background too | the layer is not window-only — the filter leaked |
// | flat red | the layer is empty: no client windows, or it was not produced |
//
// The alpha channel is the mask: `windows.a` is 0 on bare desktop and the
// surface's own premultiplied alpha inside a window, which is what makes the
// layer compositable rather than merely viewable.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

// Sorted input keys: "scene" -> 1, "win" -> 2.
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var win: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;

    if (uv.x < 0.5) {
        return vec4<f32>(textureSample(scene, samp, uv).rgb, 1.0);
    }
    // Right half: the window layer over red, so "empty" and "opaque black window"
    // stay distinguishable — the former shows red, the latter black.
    let w = textureSample(win, samp, uv);
    let bg = vec3<f32>(0.55, 0.05, 0.05);
    var col = bg * (1.0 - clamp(w.a, 0.0, 1.0)) + w.rgb;
    // Seam marker.
    if (abs(uv.x - 0.5) < 0.0012) {
        col = vec3<f32>(1.0, 1.0, 0.2);
    }
    return vec4<f32>(col, 1.0);
}
