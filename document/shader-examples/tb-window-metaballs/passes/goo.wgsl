// TB STRESS — "tb-window-metaballs". THE BUILT-IN METABALLS FIELD, BUT THE BLOBS
// ARE YOUR WINDOWS — AND SO IS THE SUBSTANCE.
//
// `shader.builtin/shaders/metaballs.wgsl` scatters wandering blobs on a jittered
// infinite grid and fuses whatever touches. This keeps the fusing and throws the
// grid away: every SOURCE is a live window rect. Drag two windows together and
// the field between them necks, bridges, and finally merges into one body; pull
// them apart and it snaps back into two. Nothing about that is faked with a
// distance test — it is the same finite-support kernel summed over rectangles
// instead of points, so the bridging is what the field does, not a special case.
//
// THE WINDOWS ARE THE MATERIAL, NOT A LAYER ON TOP
// ------------------------------------------------
// The obvious build — draw the field, then blit each window over it — gives you
// hard rectangles floating on goo, and the eye reads two separate things. So the
// windows are not composited as rectangles at all. Each window's CONTENT is
// weighted by that window's own field contribution and the weights are
// normalised, which means:
//
//   * inside a window, its weight dominates → you see that window, as usual;
//   * in the neck BETWEEN two windows, both weights are comparable → the two
//     windows' pixels cross-fade into each other, so the bridge is made of both
//     of their contents rather than of tint;
//   * past a window's edge but still inside the blob, the texture is sampled
//     clamped, so the edge row extrudes outward and the content SMEARS into the
//     bulge instead of stopping at a rectangle.
//
// One consequence worth stating: the blend is by field weight, NOT by stacking
// order. Two overlapping windows in the neck region mix rather than occlude. That
// is the effect, but it does mean this bundle is a toy — you cannot read text in
// a merged region, by construction.
//
// WHY THIS NEEDS `windows: "world"`
// ---------------------------------
// The field has to be drawn UNDER the windows and the content mixed INTO it,
// which means the background must still exist when the field is computed. An
// after-content pass is too late: the engine has already flattened each window
// into `content` and the pixels behind them are gone. So this pass owns world
// compositing (§8d) — nothing else can express a window that is partly liquid.
//
// `world` rather than `pipeline` because `pipeline` leaves iced-world panels with
// the engine, which draws them after this pass and therefore over the goo, with
// no say in the matter. Owning them does not make them meltable — see the note on
// the second loop for why a merged field has no "between" to put them in — but it
// does put the decision here rather than nowhere.
//
// WHY IT SUPPRESSES CHROME
// ------------------------
// `decorations: "off"` and `letterbox: "always"`. Both are opaque things the
// ENGINE paints around a window, and both sit exactly where this effect lives:
//
//   * the decoration border is drawn OUTSIDE the slot, so it lands on top of the
//     neck between two windows — the one part of the picture worth looking at,
//     and it would draw a crisp straight line across molten content;
//   * the letterbox fill is opaque black over the slot a client underfills, so
//     the melt would sample black and the blob would fill with a dark rectangle.
//
// Switch either back on in `pipeline.json` to see what it costs.
//
// | What you see | Meaning |
// |---|---|
// | windows melting into each other through a shared neck | working |
// | crisp rectangles floating over a blob field | content is not being weighted — check `accw` |
// | round blobs that ignore window position | rects UBO not bound (`needs`) |
// | field present, windows missing | no descriptor indexing — see the fallback |
// | hard black rectangles inside the field | `letterbox` did not take effect |
// | crisp coloured frames on each window | `decorations` did not take effect |
//
// COST. Every window within reach of a pixel is sampled for that pixel, so this
// is O(windows near me) texture reads per pixel, not O(1). The `w <= 0` skip
// bounds it to the ones whose kernel actually covers the pixel — which is why
// `reach` is a performance knob as much as a look knob.
//
// @prop reach   float default=1.0 min=0.2 max=2.5  step=0.05 label="Reach"          group="Window metaballs"
// @prop merge   float default=1.0 min=0.2 max=2.5  step=0.05 label="Merge softness" group="Window metaballs"
// @prop melt    float default=1.0 min=0.0 max=2.0  step=0.05 label="Melt"           group="Window metaballs"
// @prop glow    float default=1.0 min=0.0 max=2.0  step=0.05 label="Glow"           group="Window metaballs"
// @prop hue     float default=0.0 min=0.0 max=1.0  step=0.01 label="Palette shift"  group="Window metaballs"
// @prop wobble  float default=1.0 min=0.0 max=2.0  step=0.05 label="Wobble"         group="Window metaballs"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

// The 32-byte header is NOT optional: `_pad: vec3<u32>` is 16-byte aligned, so
// `rects` starts at offset 32 and the engine writes it there. Trimming the pad
// silently shifts every rect by one window.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    // x = kind: 0 = client window, 1 = iced-world panel. y = element alpha.
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;
@group(0) @binding(0) var samp: sampler;

// Signed distance to a rounded rectangle, in aspect-corrected uv. `b` is the
// half-extent. Negative inside.
fn sd_round_rect(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// The same monotone dark -> teal -> crest walk the built-in metaballs uses, so
// the two read as one family. Not wrapped: a wrap paints a hard contour ring at
// every blob edge.
fn palette(x: f32, shift: f32) -> vec3<f32> {
    let a = vec3<f32>(0.014, 0.026, 0.040);
    let b = vec3<f32>(0.045, 0.100, 0.130);
    let cool = vec3<f32>(0.115, 0.155, 0.205);
    let violet = vec3<f32>(0.175, 0.110, 0.215);
    let crest = mix(cool, violet, clamp(shift, 0.0, 1.0));
    let lo = mix(a, b, smoothstep(0.0, 0.55, x));
    return mix(lo, crest, smoothstep(0.5, 1.0, x));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    // Aspect correction, so a blob around a square window is round rather than
    // stretched with the display.
    let aspect = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let p = uv * aspect;

    let reach = pc.params[0].x;
    let merge = pc.params[0].y;
    let melt = pc.params[0].z;
    let glow = pc.params[0].w;
    let hue = pc.params[1].x;
    let wobble = pc.params[1].y;

    // Support radius of the kernel, in aspect-corrected uv. Everything past it
    // contributes EXACTLY zero with zero slope, which is what keeps a window
    // leaving the screen from popping a seam as it goes.
    let R = 0.10 * max(reach, 0.01);

    let n = min(windows.count, 256u);
    var field = 0.0;
    // Field-weighted window content. `acc` is premultiplied by weight; `accw` is
    // the weight sum, so `acc / accw` is the normalised mix — that normalisation
    // is what makes the neck a BLEND of two windows rather than a sum that blows
    // out to white wherever two fields overlap.
    var acc = vec3<f32>(0.0);
    var accw = 0.0;

    for (var i = 0u; i < n; i = i + 1u) {
        // Only client windows melt. An iced-world panel is composited plainly in
        // the second loop below — see the note there about what that costs.
        if (windows.attrs[i].x > 0.5) {
            continue;
        }
        let r = windows.rects[i];
        let s = windows.srcs[i];
        let centre = (r.xy + r.zw * 0.5) * aspect;
        let half = r.zw * aspect * 0.5;
        // Per-window breathing, so a still desktop is not a still picture. Phase
        // off the index: neighbours pulse against each other, which is when the
        // neck between them is most obviously alive.
        let breathe = 1.0 + 0.18 * wobble * sin(t * 0.9 + f32(i) * 2.3);
        let d = sd_round_rect(p - centre, half * breathe, min(half.x, half.y) * 0.35);
        // Wyvill-style finite support: w^3 is C2-smooth and reaches zero AT R.
        // An inverse-square sum never vanishes, so distant windows would tint the
        // whole screen and the surface threshold would drift with window count.
        let w = 1.0 - clamp(d / R, 0.0, 1.0);
        if (w <= 0.0) {
            // Out of this window's reach — and skipping the texture read is the
            // only thing keeping the cost proportional to NEARBY windows rather
            // than to every window on the desktop.
            continue;
        }
        let w3 = w * w * w;
        field = field + w3;

        // CLAMPED into the window. Inside, this is the ordinary mapping; outside,
        // the edge row extrudes outward, so the content smears into the bulge
        // instead of stopping dead at the rectangle. That extrusion IS the melt.
        //
        // `win_tex[i]` is indexed by the LOOP COUNTER only. A non-uniform index
        // into a binding_array is undefined behaviour here and showed up, once,
        // as every window sampling a neighbour's texture.
        let local = clamp((uv - r.xy) / max(r.zw, vec2<f32>(0.0001)), vec2<f32>(0.0), vec2<f32>(1.0));
        let px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        // Weight by the field AND by the window's own alpha, so a translucent
        // client contributes proportionally rather than punching an opaque hole
        // in the blend.
        let cw = w3 * clamp(px.a, 0.0, 1.0);
        acc = acc + px.rgb * cw;
        accw = accw + cw;
    }

    // Threshold the field into blob mass. `merge` sets the boundary width: crisp
    // reads as separate shells cleanly joining, soft melts into lava-lamp haze.
    let edge = mix(0.04, 0.30, clamp((merge - 0.2) / 2.3, 0.0, 1.0));
    let surf = 0.34;
    let dense = smoothstep(surf - edge, surf + edge, field);
    let core = smoothstep(surf + edge, surf + edge + 0.8, field);

    // The goo itself, which is what shows where there is field but no content —
    // the rim, and the thin part of a neck.
    var col = palette(dense, hue);
    col = col + vec3<f32>(0.10, 0.14, 0.20) * pow(core, 1.6) * 0.7 * glow;
    let bg = mix(vec3<f32>(0.012, 0.020, 0.032), vec3<f32>(0.008, 0.012, 0.022), uv.y);
    col = max(col, bg);

    // Mix the window material in. Guarded on `accw` because the normalisation
    // divides by it — with no contributing window this whole term is skipped
    // rather than producing a division by ~0 and a screen of NaN.
    if (accw > 1e-5) {
        let material = acc / accw;
        // How far the content reaches out of its own rectangle. `melt` at 0 keeps
        // content to roughly the window body; higher lets it flow to the rim.
        let lo = mix(0.85, 0.10, clamp(melt * 0.5, 0.0, 1.0));
        let presence = smoothstep(lo, 1.0, dense);
        col = mix(col, material, presence);
    }

    // A crest along the surface, so the merged silhouette reads as one body.
    // `dense * (1 - dense)` peaks exactly on the boundary and vanishes on both
    // sides, which is what draws the neck rather than outlining each window.
    let rim = dense * (1.0 - dense) * 4.0;
    col = col + vec3<f32>(0.10, 0.16, 0.22) * rim * glow;

    // The rest of the world band — iced-world panels — composited in published
    // order, so they stack correctly AMONG THEMSELVES.
    //
    // They cannot stack correctly against the windows, and that is a property of
    // the effect rather than a shortcut: the field merges every window into one
    // body with a single surface, so "between window A and window B" has no
    // pixel to name. Panels therefore land above all of it. `windows: "world"` is
    // still what this bundle wants — under `pipeline` the ENGINE draws these,
    // after this pass, which puts them on top too but with no say in the matter
    // and no way to tint, warp or omit them.
    for (var i = 0u; i < n; i = i + 1u) {
        if (windows.attrs[i].x < 0.5) {
            continue;
        }
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x < lo.x || uv.x > hi.x || uv.y < lo.y || uv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let lp = (uv - lo) / max(r.zw, vec2<f32>(0.0001));
        let pp = textureSampleLevel(win_tex[i], samp, s.xy + lp * s.zw, 0.0);
        let pa = clamp(pp.a, 0.0, 1.0) * windows.attrs[i].y;
        col = col * (1.0 - pa) + pp.rgb * windows.attrs[i].y;
    }

    return vec4<f32>(col, 1.0);
}
