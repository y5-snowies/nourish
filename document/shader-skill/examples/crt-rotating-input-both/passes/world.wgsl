// TB STRESS 15 — "tb-window-owned". THE §8d DEMONSTRATOR.
//
// `"windows": "world"` at the manifest root tells the engine NOT to draw the
// world band at all. It still publishes the world set (rects, src crops, the
// per-entry `attrs`, the bindless texture array) — this pass composites the band
// itself, in ONE before-content pass that also draws the background.
//
// WHY `world` AND NOT `pipeline`
// -----------------------------
// `pipeline` hands over the client windows only. The iced-world drawables that
// interleave with them — placeholders, group frames — stay with the engine, which
// draws them AFTER this pass. Since this pass produces one opaque fullscreen
// image, "after" means "on top of every window in it", whatever their real depth.
// `world` hands over the whole band in `drawable_order()`, and because the set is
// already back-to-front, compositing `0..count` in order reproduces the engine's
// own stacking exactly — which is the baseline this bundle then breaks on purpose
// by tilting each window.
//
// Drag a placeholder between two windows to check it: under `world` it stays
// between them. Switch this bundle to `pipeline` (or look at `tb-window-none`,
// which is deliberately left there) and it jumps in front of both.
//
// WHAT THIS PROVES, THAT NOTHING ELSE HERE CAN
// --------------------------------------------
// Every other window bundle can only paint OVER windows, because by the time an
// after-content pass runs, the engine has already blitted each window into
// `content` and destroyed the background behind it. Here the background is never
// overdrawn, so the windows can be:
//
//   * ROTATED  — they sway gently, and you can see the background in the wedges
//                the rotation vacates. Impossible with a flattened `content`.
//   * TRANSLUCENT — the grid shows THROUGH each window. Also impossible: there
//                was nothing behind them to reveal.
//
// If you see rotated, see-through windows over an unbroken grid, pipeline-owned
// window compositing works. If windows vanish entirely, the pass is not drawing
// them (check that the device reports descriptor_indexing). If they are upright
// and opaque, the engine is still blitting them — `windows: world` did not
// take effect.
//
// Being a before-content pass with no `content` input, this bundle needs no
// offscreen path at all: the whole world band is produced inside the graph. That
// is exactly the shape that becomes worker-offloadable once client-dmabuf import
// lands (SHADER_PIPELINE_WORKER.md stage 5) — today it declares `needs`, so the
// placement verdict is correctly `None`.
//
// @prop tilt  float default=0.05 min=0.0 max=0.5 step=0.005 label="Window tilt"    group="Owned windows"
// @prop alpha float default=0.82 min=0.1 max=1.0 step=0.01  label="Window opacity" group="Owned windows"

enable wgpu_binding_array;

#import crt_both::warp::sway

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // 0.x = tilt, 0.y = alpha
};
var<immediate> pc: Push;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    // x = kind: 0 = client window, 1 = iced-world panel (placeholder, group).
    // y = the element alpha the engine would have blitted it with.
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;
@group(0) @binding(0) var samp: sampler;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let aspect = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let tilt = pc.params[0].x;
    let opacity = pc.params[0].y;

    // A bold grid so it is obvious what sits BEHIND each window.
    let g = fract(uv * vec2<f32>(24.0, 14.0));
    let line = step(0.94, max(g.x, g.y));
    var col = mix(vec3<f32>(0.05, 0.07, 0.12), vec3<f32>(0.10, 0.16, 0.26), uv.y);
    col = col + vec3<f32>(0.10, 0.22, 0.30) * line;

    // Composite the whole world band ourselves, back to front, each window on its
    // own tilt. `windows: "world"`, so the set is EVERY world drawable in
    // `drawable_order()` — the panels included. Compositing them here, at their
    // own index, is the only way they land at their real depth: anything the
    // engine still draws happens after this pass and would sit on top of every
    // window regardless.
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let s = windows.srcs[i];
        // A panel is not a client window and gets none of the effect: plain
        // axis-aligned blit, exactly what the engine would have drawn, just in
        // the right place in the stack.
        if (windows.attrs[i].x > 0.5) {
            let lo = r.xy;
            let hi = r.xy + r.zw;
            if (uv.x >= lo.x && uv.x <= hi.x && uv.y >= lo.y && uv.y <= hi.y) {
                let lp = (uv - lo) / max(r.zw, vec2<f32>(0.0001));
                let pp = textureSampleLevel(win_tex[i], samp, s.xy + lp * s.zw, 0.0);
                let pa = clamp(pp.a, 0.0, 1.0) * windows.attrs[i].y;
                col = col * (1.0 - pa) + pp.rgb * windows.attrs[i].y;
            }
            continue;
        }
        let centre = r.xy + r.zw * 0.5;
        // Per-window sway, from the SHARED definition the pointer warp inverts.
        // Not a copy of the same expression: a copy is how the picture and the
        // cursor drift apart, and the drift is invisible until you click.
        let a = sway(tilt, t, f32(i));
        let ca = cos(-a);
        let sa = sin(-a);
        // Inverse-rotate the pixel into this window's local frame.
        let p = (uv - centre) * aspect;
        let rp = vec2<f32>(p.x * ca - p.y * sa, p.x * sa + p.y * ca) / aspect;
        let local = rp / max(r.zw, vec2<f32>(0.0001)) + vec2<f32>(0.5);
        if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
            continue;
        }
        let px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        // The window's own premultiplied alpha, scaled by the opacity prop, so the
        // grid stays visible through it.
        let a_eff = clamp(px.a, 0.0, 1.0) * opacity;
        col = col * (1.0 - a_eff) + px.rgb * opacity;
        // Edge so a fully transparent window is still locatable.
        let e = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y));
        if (e * min(r.zw.x * res.x, r.zw.y * res.y) < 2.5) {
            col = vec3<f32>(1.0, 0.85, 0.25);
        }
    }
    return vec4<f32>(col, 1.0);
}
