// TB STRESS 24 — "tb-window-frost". Glass, WITHOUT the bindless texture array.
//
// Frosts the wallpaper behind every window and composites the window over it —
// the `glass` effect — but built from SEPARATED LAYERS rather than the bindless
// texture array: `"windows": "world"` keeps the world band out of `content`, so
// `content` is the background alone and the `windows` layer carries the windows.
// It therefore:
//
//   * needs NO descriptor indexing, so it runs on devices where `glass` falls
//     back to the stock background entirely;
//   * needs no per-window texture array, so nothing is index-aligned and the
//     whole class of "each window shows the other window" bugs cannot occur;
//   * works identically for dmabuf and SHM clients, because the compositor has
//     already resolved both into the layer.
//
// That combination is the argument for sharing the LAYER with the worker rather
// than importing client buffers (worker plan §5b): if this looks as good as
// `glass`, the cheap route covers the real cases.
//
// TESTED LIMIT: this does NOT show windows below other windows through the frost,
// where `glass` does. What lies behind window i is the background plus windows
// 0..i-1, and a flattened layer only offers background-plus-all or
// background-alone — never the prefix. Flattening destroys the ordering, so this
// is not fixable here; it is why per-window textures remain necessary.
//
// The trade is real and worth knowing: a flattened layer can only be sampled
// where the window actually is. An effect that reads window i from OUTSIDE window
// i's rect — a thumbnail, a reflection cast elsewhere — still needs the per-window
// array. Frosting reads in-rect, so it does not.
//
// @prop blur  float default=26.0 min=0.0 max=96.0 step=1.0  label="Frost radius px" group="Frost"
// @prop tint  float default=0.85 min=0.0 max=1.5  step=0.01 label="Backdrop tint"   group="Frost"

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

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;

// Disk blur of the SCENE. This only frosts anything because the bundle declares
// `"windows": "world"`: the engine then leaves the whole world band OUT of `content`,
// so behind a window `scene` really is the wallpaper and the windows arrive
// separately through the layer.
//
// WITHOUT that flag `content` is the FULLY composited scene — windows included —
// so inside a window this would blur the window itself, and compositing the window
// back over its own blur is a no-op. That is exactly what the first version of
// this bundle did: it looked like a plain translucent window and frosted nothing.
fn frost(uv: vec2<f32>, res: vec2<f32>, radius_px: f32) -> vec3<f32> {
    let r = radius_px / max(res, vec2<f32>(1.0));
    var acc = textureSampleLevel(scene, samp, uv, 0.0).rgb;
    var w = 1.0;
    for (var i = 0; i < 12; i = i + 1) {
        let a = f32(i) * 0.5235988;
        let dir = vec2<f32>(cos(a), sin(a));
        acc = acc + textureSampleLevel(scene, samp, uv + dir * r, 0.0).rgb;
        acc = acc + textureSampleLevel(scene, samp, uv + dir * r * 0.5, 0.0).rgb;
        w = w + 2.0;
    }
    return acc / w;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let blur = pc.params[0].x;
    let tint = pc.params[0].y;

    // Inside any window? Rects are plain numbers — no array indexing needed.
    var inside = false;
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (uv.x >= r.x && uv.x <= hi.x && uv.y >= r.y && uv.y <= hi.y) {
            inside = true;
        }
    }
    if (!inside) {
        return vec4<f32>(textureSampleLevel(scene, samp, uv, 0.0).rgb, 1.0);
    }

    // The window's own pixels, straight from the layer — premultiplied, so this
    // is a plain "over" onto the frosted backdrop.
    let w = textureSampleLevel(win, samp, uv, 0.0);
    let backdrop = frost(uv, res, blur) * tint;
    return vec4<f32>(w.rgb + backdrop * (1.0 - clamp(w.a, 0.0, 1.0)), 1.0);
}
