// Built-in background: "Black" — opaque black, and nothing else.
//
// This is a MEASUREMENT BASELINE, not a look. Every other built-in spends real
// fragment ALU (the cheapest, papercut, is still ~60-90 ops/pixel; anything with
// an `fbm()` call is 360+), so timing them tells you the shader's cost tangled
// up with everything around it. This one is ~1 op/pixel, which makes it the
// control: whatever frame time remains with this selected is the compositor's
// own overhead — the composite, the flip, the pacing — with the shader removed
// from the picture.
//
// Deliberately no `@prop` lines: there is nothing to tune, and an empty control
// group in the settings panel would only invite the question.
//
// It still declares the full `Push` block and honours `alpha`/`srgb` so it sits
// on exactly the same contract as every other built-in — a baseline that skipped
// the shared plumbing would not be measuring the same path.

struct Push {
    res_zoom_time: vec4<f32>,        // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,             // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,           // x = lock_amount, y = alpha, z = srgb flag
    params: array<vec4<f32>, 4>,     // shader-authored @prop values (16 floats)
};
var<immediate> pc: Push;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Premultiplied, like every other built-in: the fullscreen pipeline blends
    // src + dst*(1-srcA), so the colour must already be scaled by alpha.
    let a = clamp(pc.lock_alpha.y, 0.0, 1.0);
    return vec4<f32>(0.0, 0.0, 0.0, a);
}
