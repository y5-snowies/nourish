// vignette pass 1/2 — "backdrop" (before-content): a calm vertical gradient with
// faint drifting noise, drawn as the background (into `content`). Windows then
// composite on top, and the after-content `vignette` pass darkens the edges of
// the whole thing. Fragment-only; the engine pairs the fullscreen vertex.
//
// @prop hue float default=0.58 min=0.0 max=1.0 step=0.01 label="Backdrop hue" group="Vignette"

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, w = time
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // @prop slot 0 = hue
};
var<immediate> pc: Push;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let hue = pc.params[0].x; // @prop hue
    let uv = frag.xy / res;

    let top = mix(vec3<f32>(0.05, 0.07, 0.13), vec3<f32>(0.13, 0.10, 0.20), hue);
    let bot = vec3<f32>(0.02, 0.03, 0.06);
    var col = mix(top, bot, uv.y);

    // Faint drifting grain so the gradient isn't flat.
    let n = hash(floor((uv + t * 0.01) * 40.0));
    col = col + (n - 0.5) * 0.02;

    return vec4<f32>(col, 1.0);
}
