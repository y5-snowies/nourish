// TB STRESS 18 — "tb-window-chroma". Per-window auto chroma key.
//
// Estimates each window's DOMINANT background colour, then punches every pixel
// within +-tolerance of it out to transparent, so the wallpaper shows through the
// window's chrome while its text and content stay solid.
//
// THIS BUNDLE CANNOT BE WRITTEN WITHOUT §8d. Making a window pixel transparent
// requires something behind it to reveal, and the engine's flattened `content`
// has already overdrawn the background. So it declares `"windows": "world"`:
// the engine leaves client windows out of the world band and this pass composites
// them itself, over a background it drew moments earlier and therefore still has.
//
// Dominant-colour estimate: 9 probes in a 3x3 grid at 8% / 50% / 92% of the
// window — the corners and edge midpoints land on chrome, which is what we want
// to key out; the centre probe keeps a full-bleed window from keying its content.
// The mode is the probe with the most neighbours inside `tol`, so a window whose
// chrome is a gradient still resolves to its dominant band rather than an average
// nobody actually has.
//
// Deliberately brute force: the 9 probes and their 81 comparisons are recomputed
// for every pixel of every window, even though the answer is constant per window.
// A reduction pass would need a per-window target, which the manifest's
// output-fraction targets cannot express. That redundancy is the point for a
// stress bundle — it is a realistic shape for "expensive per-window analysis".
//
// All `win_tex` reads use the loop counter (wave-uniform) — SHADER_PIPELINE.md §3.
//
// @prop tol   float default=0.13 min=0.01 max=0.60 step=0.01 label="Key tolerance"  group="Chroma"
// @prop soft  float default=0.06 min=0.00 max=0.30 step=0.01 label="Edge softness"  group="Chroma"
// @prop keep  float default=0.10 min=0.00 max=1.00 step=0.01 label="Residual alpha" group="Chroma"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;

// `_pad: vec3<u32>` is 16-byte aligned, so the arrays start at offset 32. Keep it.
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

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let tol = pc.params[0].x;
    let soft = pc.params[0].y;
    let keep = pc.params[0].z;

    // A busy wallpaper, so a successful key is obvious: the pattern must be
    // visible THROUGH the window chrome, not merely around it.
    let g = fract(uv * vec2<f32>(16.0, 10.0));
    let grid = step(0.93, max(g.x, g.y));
    var col = mix(vec3<f32>(0.04, 0.09, 0.16), vec3<f32>(0.16, 0.05, 0.14),
                  0.5 + 0.5 * sin((uv.x + uv.y) * 3.0 + t * 0.2));
    col = col + vec3<f32>(0.10, 0.30, 0.35) * grid;

    // Composite the whole world band BACK TO FRONT, keying as we go. Under
    // `windows: "world"` the set includes iced-world panels; they are not client
    // windows, so they are blitted plainly rather than keyed — chroma-keying a
    // placeholder would dissolve it against its own flat fill.
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x < lo.x || uv.x > hi.x || uv.y < lo.y || uv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let local = (uv - lo) / max(r.zw, vec2<f32>(0.0001));
        if (windows.attrs[i].x > 0.5) {
            let pp = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
            let pa = clamp(pp.a, 0.0, 1.0) * windows.attrs[i].y;
            col = col * (1.0 - pa) + pp.rgb * windows.attrs[i].y;
            continue;
        }

        // 9 probes: grid positions 0.08 / 0.50 / 0.92 on each axis.
        var probe: array<vec3<f32>, 9>;
        for (var k = 0u; k < 9u; k = k + 1u) {
            let p = vec2<f32>(0.08 + f32(k % 3u) * 0.42, 0.08 + f32(k / 3u) * 0.42);
            probe[k] = textureSampleLevel(win_tex[i], samp, s.xy + p * s.zw, 0.0).rgb;
        }
        // Mode: the probe with the most neighbours within `tol`.
        var dominant = probe[0];
        var best = -1;
        for (var a = 0u; a < 9u; a = a + 1u) {
            var votes = 0;
            for (var b = 0u; b < 9u; b = b + 1u) {
                if (distance(probe[a], probe[b]) < tol) {
                    votes = votes + 1;
                }
            }
            if (votes > best) {
                best = votes;
                dominant = probe[a];
            }
        }

        let px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        // Distance from the key colour drives alpha: inside the tolerance goes
        // (nearly) clear, outside stays solid, `soft` feathers the boundary so
        // anti-aliased glyph edges do not fringe.
        let d = distance(px.rgb, dominant);
        let keyed = smoothstep(tol, tol + max(soft, 0.001), d);
        let a = clamp(px.a, 0.0, 1.0) * mix(keep, 1.0, keyed);
        col = col * (1.0 - a) + px.rgb * a;
    }
    return vec4<f32>(col, 1.0);
}
