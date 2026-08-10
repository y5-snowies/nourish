// TB STRESS 17 — "tb-window-none". THE NEGATIVE CONTROL for §8d.
//
// Claims full window-compositing responsibility (`"windows": "pipeline"`) and
// then draws NO WINDOWS AT ALL. The desktop should show this background and
// nothing else — no window content anywhere, however many clients are open.
//
// ANY window pixel on screen means something is still blitting windows behind
// the pipeline's back: `skip_windows` is not suppressing every path (HDR branch,
// a second draw loop, a screen-band element mis-tagged as a window, ...).
// That is the whole point of the bundle — it isolates "did the engine really
// stop drawing" from "did the pipeline draw them correctly", which every other
// window bundle conflates.
//
// LIVENESS MARKER — read this first.
// A bundle that silently failed to load would ALSO show a bare desktop, so a
// blank screen alone proves nothing. The top-left blocks are the disambiguator:
// one white block per window in `windows.count`, drawn from the window-rects UBO.
//
//   blocks present, no window content  ->  suppression works. PASS.
//   blocks present, window content     ->  MIS-BLIT: something else drew them.
//   no blocks at all                   ->  the bundle never loaded; not a valid
//                                          run (check the log for a warn!).
//
// The count also tells you the engine is still COLLECTING the world set while
// declining to draw it — which is exactly the contract `windows: pipeline` makes.
//
// WHY THIS ONE STAYS ON `pipeline`
// -------------------------------
// Every other bundle in the family moved to `windows: "world"`. This one is left
// behind on purpose, and it is the cheapest way to SEE the difference: `pipeline`
// suppresses client windows only, so iced-world placeholders and group frames are
// still drawn by the engine — after this pass, hence on top of a background that
// claims to own everything. Open a placeholder here and it is visible; open one
// under `tb-window-owned` and the bundle decides where it goes.
//
// Which also means: window content on screen is a FAILURE here, but a placeholder
// on screen is expected. Read the table above with that in mind.
//
// Deliberately needs only `window-rects`, not `window-textures`: rects are a
// plain UBO, so this test does not depend on descriptor indexing and runs on
// devices where the bindless bundles fall back.
//
// This pass has ZERO inputs while binding the window set — the case that used to
// build a pipeline layout with no set 0 and fault the GPU. It is a regression
// test for that too.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

// Header note: `_pad: vec3<u32>` has SIXTEEN-byte alignment, so the arrays start
// at offset 32. Keep it exactly as written — see SHADER_PIPELINE.md §3.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    // Unmistakable background: diagonal sweep. If windows were being blitted,
    // their rectangles would break this pattern instantly.
    let d = fract((uv.x + uv.y) * 6.0 - t * 0.15);
    var col = mix(vec3<f32>(0.06, 0.05, 0.12), vec3<f32>(0.14, 0.09, 0.22), d);
    col = col + vec3<f32>(0.05, 0.10, 0.14) * step(0.97, d);

    // Liveness: one white block per known window. NOT window content — this is
    // read from the rects UBO, so it proves the pass ran and the engine is still
    // collecting the window set while declining to draw it.
    let n = min(windows.count, 256u);
    if (uv.y < 0.03) {
        let slot = u32(floor(uv.x * 40.0));
        if (slot < n && fract(uv.x * 40.0) < 0.75) {
            col = vec3<f32>(1.0);
        }
    }
    return vec4<f32>(col, 1.0);
}
