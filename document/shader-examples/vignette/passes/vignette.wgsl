// Phase-2 demo — a single AFTER-CONTENT pass. It samples the built-in `content`
// target (the fully composited scene: background + all windows) and darkens the
// edges. Because it runs after window compositing, the vignette sits ABOVE the
// windows — the requirement multipass after-content unlocks.
//
// `inputs: { scene: content }` → the sorted binding name "scene" binds at 1, and
// `content` is the composited scene (input sentinel, bound by the engine).
//
// @prop amount float default=0.75 min=0.0 max=1.0 step=0.01 label="Vignette amount" group="Vignette"
// @prop radius float default=0.80 min=0.1 max=1.5 step=0.01 label="Radius"          group="Vignette"

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, w = time
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // @prop slots: 0 = amount, 1 = radius
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let uv = frag.xy / res;
    let amount = pc.params[0].x; // @prop amount
    let radius = pc.params[0].y; // @prop radius

    let col = textureSample(scene, samp, uv).rgb;

    // Aspect-correct distance from centre → smooth edge falloff.
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let d = length(p);
    let v = smoothstep(radius, radius * 0.35, d); // 1 at centre, 0 at the corners
    let vig = mix(1.0 - amount, 1.0, v);

    return vec4<f32>(col * vig, 1.0);
}
