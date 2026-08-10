// TRANSPORTED `windows`, on the worker. The other new row.
//
// `windows` is exported, not derived — deliberately the opposite choice from
// `history`. The worker holds per-window textures and rects, so it COULD
// composite a window band itself. That band would not be the engine's: no
// decorations, no subsurfaces, no popups, no z-order beyond the array's, none of
// the per-surface AA or HDR the engine applies. A pass that asks for `windows`
// means the layer the compositor actually drew, so it gets that image.
//
// The layer is consumption-gated on BOTH sides: the compositor allocates it only
// when a bundle names it, and exports it only when it allocates. A bundle that
// never samples `windows` pays for none of this.
//
// This pass tints strictly where the layer says a window is — using the layer's
// ALPHA, which is exactly the information a shader could not reconstruct from
// rects alone (rects are slots; the layer is what was drawn into them).
//
// | What you see | Meaning |
// |---|---|
// | windows tinted green, background untouched | transported layer works |
// | whole screen tinted | the layer sampled null/opaque — alpha is not arriving |
// | nothing tinted | `windows` bound null, or the export never published |
// | tint follows window SLOTS but not their content shape | you are looking at rects, not the layer |
//
// @prop tint float default=1.0 min=0.0 max=2.0 step=0.05 label="Window tint" group="Worker windows"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var win: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let tint = pc.params[0].x;
    let base = textureSample(scene, samp, uv).rgb;
    // The layer is cleared to TRANSPARENT, so alpha IS the window mask.
    let a = clamp(textureSample(win, samp, uv).a, 0.0, 1.0);
    let marked = base * vec3<f32>(0.55, 1.35, 0.65);
    return vec4<f32>(mix(base, marked, a * clamp(tint, 0.0, 2.0)), 1.0);
}
