// The shared BACKGROUND pass for the `Multipass` set: the stock parallax, drawn
// into whatever target the bundle points it at.
//
// Every bundle in the category references THIS file rather than copying it
// (`"shader": "../mp-parallax/passes/backdrop.wgsl"`), which is what makes them
// all genuinely the same background with a different effect on top — and what
// makes "the background changed" one edit instead of seven.
//
// The three knobs are the stock parallax's own, under its own names, so a world
// switching between these bundles keeps its tuning: the per-world overrides are
// keyed BY NAME.
//
// @prop drift   float default=1.0 min=0.0 max=3.0 step=0.01 label="Drift speed"   group="Background"
// @prop stars   float default=1.0 min=0.0 max=2.0 step=0.01 label="Star density"  group="Background"
// @prop nebula  float default=1.0 min=0.0 max=2.0 step=0.01 label="Nebula"        group="Background"

#import mp::parallax::parallax_scene

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let col = parallax_scene(
        frag.xy, res, pc.res_zoom_time.z, pc.res_zoom_time.w,
        pc.pan_flow.xy, pc.pan_flow.zw,
        pc.params[0].x, pc.params[0].y, pc.params[0].z,
    );
    return vec4<f32>(col, 1.0);
}
