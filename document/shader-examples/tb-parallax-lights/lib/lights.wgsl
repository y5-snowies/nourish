// Shared emitter definition for `tb-parallax-lights`, imported by BOTH passes.
//
// THIS FILE IS THE POINT OF THE BUNDLE. The before-content pass draws the
// emitters into the background; the after-content pass lights the windows from
// them. Neither pass is told where the lights are — both DERIVE the positions
// from `time` using the identical function here.
//
// Consequence: the two bands must agree on `time`, or the glow in the background
// and the highlight on the windows drift apart. That makes clock divergence a
// VISIBLE CORRECTNESS BUG rather than a cosmetic one — see the note in
// passes/lit.wgsl.
#define_import_path lights

// Emitter `i` (0..2) in screen-UV space at time `t`. Slow, wide Lissajous paths
// so the lights sweep across the whole output and pass behind windows.
fn light_pos(i: u32, t: f32) -> vec2<f32> {
    let fi = f32(i);
    let sp = 0.11 + fi * 0.037;
    let ph = fi * 2.3;
    return vec2<f32>(
        0.5 + 0.42 * sin(t * sp + ph),
        0.5 + 0.34 * cos(t * sp * 0.83 + ph * 1.7)
    );
}

// Emitter tint. Warm / cyan / magenta so which light is hitting a window is
// unambiguous at a glance.
fn light_tint(i: u32) -> vec3<f32> {
    if (i == 0u) { return vec3<f32>(1.00, 0.72, 0.32); }
    if (i == 1u) { return vec3<f32>(0.35, 0.85, 1.00); }
    return vec3<f32>(0.95, 0.40, 0.95);
}

// Inverse-square-ish falloff with a soft core, in aspect-corrected UV units.
fn light_falloff(d: f32, reach: f32) -> f32 {
    let x = max(d, 0.0) / max(reach, 0.0001);
    return 1.0 / (1.0 + 16.0 * x * x);
}
