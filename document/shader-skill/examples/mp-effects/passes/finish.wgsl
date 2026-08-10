// "mp-effects", pass 3/3 — everything that belongs OVER the finished desktop.
//
// WHY THIS IS ITS OWN PASS
// ------------------------
// Motion blur needs the composited picture, and under `windows: "world"` no
// engine-provided image is one: `content` is the backdrop with the whole world
// band left out, and `history` is a copy of `content`, so both know only the
// wallpaper. Blurring either smears the desk while the windows sit still on top
// of it.
//
// So `compose` writes its result to a `picture` target instead of straight to the
// swapchain, and this pass samples THAT. The blur is over windows, panels,
// backdrop and every through-the-window effect — the whole frame, which is what
// the knob always claimed to do and could not while the band was not in any image
// it could reach.
//
// FROM THE CURRENT FRAME, NOT THE PREVIOUS ONE
// --------------------------------------------
// The old blur averaged the previous frame's `content` toward the current one.
// That is a temporal blend: it ghosts, it lags by a frame, and on a fast pan it
// double-images rather than streaks, because two samples a frame apart is two
// pictures and not a smear.
//
// This smears the CURRENT picture ALONG THE PAN, which is what a camera's open
// shutter does. The direction comes from the same velocity lane the old one
// weighted itself by (`lock_alpha.w`), so the knob keeps its feel — and the
// bundle no longer needs `previous_frame` at all, which drops a full-screen copy
// per frame from a bundle that was already the most expensive one shipped.
//
// The blend stays self-normalising (`s / (1 + s)`): `speed` is in world px/s and
// ranges over orders of magnitude between a nudge and a fling, so a knob compared
// against a fixed constant is either always off or always saturated.
//
// STICKY EDGE lives here, not in `compose`, and that is the whole reason it is a
// knob and not a bug. It displaces the COMPOSITED scene near a window boundary —
// windows included, which is what makes the edge look drawn-out and tacky rather
// than refractive. In `compose` there is no composited scene yet: displacing the
// backdrop there did nothing you could see, because the band composite that runs
// straight after overwrites every pixel a window covers, which is exactly the set
// of pixels the effect acts on.
//
// It needs the rects and nothing else, so `window_geometry` without
// `window_textures` — no bindless array, no per-drawable sampling.
//
// @prop sticky   float default=0.0 min=0.0 max=1.0 step=0.01 label="Sticky edge"   group="Edges"
// @prop blur     float default=0.0 min=0.0 max=1.0 step=0.01 label="Motion blur"   group="Motion"
// @prop lights   float default=0.0 min=0.0 max=1.0 step=0.01 label="Light motes"   group="Motion"
// @prop vignette float default=0.0 min=0.0 max=1.0 step=0.01 label="Vignette"      group="Framing"

#import mp::parallax::hash2

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var picture: texture_2d<f32>;

/// Layout is fixed by the engine's UBO — rects, then srcs, then the per-entry
/// attributes — so all three are declared even though only the rects are read.
/// Dropping one would shift the others' offsets.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    let p_sticky = pc.params[0].x;
    let p_blur = pc.params[0].y;
    let p_lights = pc.params[0].z;
    let p_vig = pc.params[0].w;

    // STICKY EDGE — pull the picture along the nearest edge of the drawable under
    // this pixel. The front-most one, found back to front like `compose` does, so
    // the two agree about which window a pixel belongs to.
    var suv = uv;
    if (p_sticky > 0.0) {
        let n = min(windows.count, 256u);
        for (var k = 0u; k < n; k = k + 1u) {
            let i = n - 1u - k;
            let r = windows.rects[i];
            let hi = r.xy + r.zw;
            if (uv.x < r.x || uv.x > hi.x || uv.y < r.y || uv.y > hi.y) {
                continue;
            }
            let dl = uv.x - r.x;
            let dr = hi.x - uv.x;
            let dt = uv.y - r.y;
            let db = hi.y - uv.y;
            let d = min(min(dl, dr), min(dt, db));
            var nrm = vec2<f32>(-1.0, 0.0);
            if (d == dr) { nrm = vec2<f32>(1.0, 0.0); }
            else if (d == dt) { nrm = vec2<f32>(0.0, -1.0); }
            else if (d == db) { nrm = vec2<f32>(0.0, 1.0); }
            suv = uv + nrm * (1.0 - smoothstep(0.0, 0.02, d)) * p_sticky * 0.012;
            break;
        }
    }

    var col = textureSampleLevel(
        picture, samp, clamp(suv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0,
    ).rgb;

    // MOTION BLUR — a directional smear along the pan, over the whole composited
    // frame. A still camera does no work at all: `blend` is zero and the loop
    // never runs.
    if (p_blur > 0.0) {
        let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;
        let s = length(vel) * p_blur * 0.02;
        let blend = 0.9 * s / (1.0 + s);
        if (blend > 0.002) {
            // Along the direction of travel, centred on the pixel, so the streak
            // has the picture at both ends rather than trailing off one side.
            // Length grows with the smear: a slow pan softens, a fast one streaks.
            let dir = normalize(vel / max(length(vel), 0.0001));
            let step = dir * (blend * 30.0) / res;
            var acc = col;
            var wsum = 1.0;
            for (var i = 1; i <= 6; i = i + 1) {
                let d = step * f32(i) / 6.0;
                acc = acc + textureSampleLevel(
                    picture, samp, clamp(suv + d, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0,
                ).rgb;
                acc = acc + textureSampleLevel(
                    picture, samp, clamp(suv - d, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0,
                ).rgb;
                wsum = wsum + 2.0;
            }
            col = mix(col, acc / wsum, blend);
        }
    }

    // LIGHT MOTES — additive drifting particles over everything, on the same
    // cell-hash the parallax uses for its stars so they belong to the same scene.
    if (p_lights > 0.0) {
        for (var i = 1; i <= 2; i = i + 1) {
            let depth = f32(i);
            let sp = uv * vec2<f32>(res.x / res.y, 1.0) * (14.0 * depth)
                + vec2<f32>(t * 0.02 * depth, -t * 0.035 * depth);
            let id = floor(sp);
            let fp = fract(sp) - 0.5;
            let h = hash2(id);
            if (h > 0.965) {
                let pulse = 0.55 + 0.45 * sin(t * 1.7 + h * 90.0);
                let d = length(fp);
                let glow = smoothstep(0.16, 0.0, d) + smoothstep(0.42, 0.0, d) * 0.22;
                col = col + vec3<f32>(0.55, 0.78, 1.0) * glow * pulse * p_lights * 0.5 / depth;
            }
        }
    }

    // VIGNETTE — last, and over the windows too. That is the difference between a
    // vignette on the BACKGROUND (which the stock parallax already offers, and
    // which windows sit on top of) and one on the composited desktop.
    if (p_vig > 0.0) {
        let c = (uv - vec2<f32>(0.5)) * vec2<f32>(res.x / max(res.y, 1.0), 1.0);
        col = col * mix(1.0, smoothstep(1.05, 0.20, length(c)), p_vig);
    }

    // The one encode, on the pass that writes the swapchain.
    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}
