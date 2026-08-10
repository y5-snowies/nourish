// motion-blur pass 1/2 — "backdrop" (before-content). A bold, PAN-LINKED grid of
// glowing coloured dots drawn as the background (into `content`). Because it
// tracks the camera (world = uv - pan*0.0006, the built-in pan convention), the
// pattern slides when you pan — which is exactly what the after-content blur
// needs to smear. Windows composite on top; then `blur` smears the whole thing.
//
// @prop scale float default=10.0 min=2.0 max=30.0 step=0.5 label="Grid density" group="Motion blur"

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,          // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // @prop slot 0.x = scale
};
var<immediate> pc: Push;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let pan = pc.pan_flow.xy;
    let scale = max(pc.params[0].x, 1.0); // @prop scale

    var uv = frag.xy / max(res, vec2<f32>(1.0));
    uv.x = uv.x * (res.x / max(res.y, 1.0));

    // Camera-tracked world coords (same convention as the built-in scenes).
    let world = uv - pan * 0.0006;

    let cell = world * scale;
    let id = floor(cell);
    let f = fract(cell) - vec2<f32>(0.5);
    let d = length(f);
    let dot = smoothstep(0.35, 0.12, d);

    let hue = hash(id) * 6.2831853;
    let tint = vec3<f32>(
        0.5 + 0.5 * sin(hue + t * 0.5),
        0.5 + 0.5 * sin(hue + 2.094 + t * 0.4),
        0.5 + 0.5 * sin(hue + 4.188 + t * 0.3),
    );
    let bg = vec3<f32>(0.02, 0.03, 0.06);
    return vec4<f32>(bg + tint * (0.12 + 0.88 * dot), 1.0);
}
