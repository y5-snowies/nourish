// "mp-levels" — BRIGHTNESS, CONTRAST, EXPOSURE, globally and per window.
//
// Six props: one set applied to the whole desktop, and a second set applied ONLY
// inside window rectangles, on top of the first. That is the useful shape — lift
// the windows out of a dim background, or push the background down without
// touching the thing being read — and it needs nothing but `world_geometry`,
// which is the cheapest requirement there is: a UBO of numbers, no textures, no
// descriptor indexing.
//
// Applied in the order a colourist would: exposure first (a MULTIPLY, which is
// what a camera stop is), then contrast (a pivot around mid grey), then
// brightness (an ADD). The order is not interchangeable — adding before
// multiplying scales the offset too, so the black point drifts the moment you
// touch exposure.
//
// Exposure is in STOPS, so ±1 is a doubling or halving of the light and the
// control behaves the same at either end of its range. The multiply is
// `exp2(stops)`.
//
// WINDOW MEMBERSHIP is `world_geometry` — the whole band, iced-world panels
// included. A placeholder is a thing on the desk like any other, and lifting the
// windows while leaving the panel between them dark would look like a bug.
//
// Every default is the identity, so selecting this bundle changes nothing until a
// knob moves.
//
// @prop exposure   float default=0.0 min=-3.0 max=3.0 step=0.01 label="Exposure (stops)" group="Everything"
// @prop contrast   float default=1.0 min=0.0  max=3.0 step=0.01 label="Contrast"         group="Everything"
// @prop brightness float default=0.0 min=-0.5 max=0.5 step=0.01 label="Brightness"       group="Everything"
// @prop win_exposure   float default=0.0 min=-3.0 max=3.0 step=0.01 label="Exposure (stops)" group="Windows only"
// @prop win_contrast   float default=1.0 min=0.0  max=3.0 step=0.01 label="Contrast"         group="Windows only"
// @prop win_brightness float default=0.0 min=-0.5 max=0.5 step=0.01 label="Brightness"       group="Windows only"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

// Layout is fixed by the engine's UBO — rects, then srcs, then the per-entry
// attributes — so all three are declared even though only the rects are read.
// `attrs`, not `meta`: `meta` is a reserved WGSL keyword.
struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;

/// Exposure → contrast → brightness. Not clamped: the global pass and the
/// window pass are applied back to back, and clamping between them would crush
/// a highlight that the second stage was about to bring back down.
fn levels(c: vec3<f32>, stops: f32, contrast: f32, brightness: f32) -> vec3<f32> {
    var out = c * exp2(stops);
    out = (out - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
    return out + vec3<f32>(brightness);
}

fn in_a_window(uv: vec2<f32>) -> bool {
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (uv.x >= r.x && uv.x <= hi.x && uv.y >= r.y && uv.y <= hi.y) {
            return true;
        }
    }
    return false;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let src = textureSampleLevel(scene, samp, uv, 0.0).rgb;

    var col = levels(src, pc.params[0].x, pc.params[0].y, pc.params[0].z);

    // The window set is only worth walking when the window knobs are off their
    // identity. The comparison is against push constants, so it is uniform across
    // the draw and costs nothing when they are.
    let win_e = pc.params[0].w;
    let win_c = pc.params[1].x;
    let win_b = pc.params[1].y;
    if (win_e != 0.0 || win_c != 1.0 || win_b != 0.0) {
        if (in_a_window(uv)) {
            col = levels(col, win_e, win_c, win_b);
        }
    }

    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}
