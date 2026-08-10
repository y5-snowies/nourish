// motion-blur pass 2/2 — "blur" (AFTER-CONTENT). Drastic pan-gated smear: blend a
// SPATIALLY-BLURRED copy of the previous frame (`history`) over the current frame
// (`content`), weighted by pan speed. Two things make it read as motion:
//   1. `history` is last frame's WORLD content (background + windows), captured at
//      the PREVIOUS pan — so it is already offset in the pan direction. Blending it
//      in produces a directional ghost for free.
//   2. We disk-blur `history` with a radius that grows with pan speed, turning that
//      offset ghost into a smooth smear rather than a sharp double image.
// A still camera → speed ~0 → blend 0 → the current frame, untouched.
//
// inputs: { cur: content, prev: history } — sorted names bind cur->1, prev->2.
// The 2D pan velocity arrives packed in the engine push `lock_alpha.w` lane; this
// pass only needs its magnitude, but the direction is there for a directional smear.
//
// NOTE: settings sliders do not yet drive multipass @props, so these DEFAULTS are
// what runs — they are tuned AGGRESSIVE on purpose. Lower `strength`/`max_blend`/
// `radius` in this file to dial it back.
//
// @prop strength  float default=0.10 min=0.0 max=1.0   step=0.005 label="Pan sensitivity" group="Motion blur"
// @prop max_blend float default=0.92 min=0.0 max=0.98  step=0.01  label="Max smear"       group="Motion blur"
// @prop radius    float default=48.0 min=0.0 max=128.0 step=1.0   label="Smear radius px"  group="Motion blur"

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,        // x = lock, y = alpha, z = srgb, w = PACKED PAN VELOCITY
    params: array<vec4<f32>, 2>,  // @prop: 0.x=strength, 0.y=max_blend, 0.z=radius
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var cur: texture_2d<f32>;
@group(0) @binding(2) var prev: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let uv = frag.xy / max(res, vec2<f32>(1.0));

    // The engine packs the 2D pan velocity into the single `lock_alpha.w` lane as
    // two snorm16 halves scaled by VELOCITY_LANE_SCALE (see two.draw/draw.vulkan).
    let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;
    let speed = length(vel); // world px/s, ~0 when still
    let strength = pc.params[0].x;         // @prop strength
    let max_blend = pc.params[0].y;        // @prop max_blend
    let radius_px = pc.params[0].z;        // @prop radius

    // Self-normalising saturation: unit-agnostic sensitivity knob.
    let s = speed * strength;
    let blend = max_blend * s / (1.0 + s);

    let cur_c = textureSample(cur, samp, uv).rgb;

    // Disk blur of the previous frame; radius grows with the smear amount so a
    // still camera does no work and a fast pan smears hard. 24 taps (2 rings).
    let r = (radius_px * blend) / max(res, vec2<f32>(1.0));
    var acc = textureSample(prev, samp, uv).rgb;
    var wsum = 1.0;
    for (var i = 0; i < 12; i = i + 1) {
        let a = f32(i) * 0.5235988; // 30°
        let dir = vec2<f32>(cos(a), sin(a));
        acc = acc + textureSample(prev, samp, uv + dir * r).rgb;
        acc = acc + textureSample(prev, samp, uv + dir * r * 0.5).rgb;
        wsum = wsum + 2.0;
    }
    let prev_blur = acc / wsum;

    return vec4<f32>(mix(cur_c, prev_blur, blend), 1.0);
}
