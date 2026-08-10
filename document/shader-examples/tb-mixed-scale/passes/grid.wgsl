// TB STRESS 10/10 — "tb-mixed-scale", pass 1/3: a fine 1px grid at FULL res.
// A 1px grid is the harshest possible input for a downsample/upsample round trip
// — any UV or half-texel error turns it into moire immediately.
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = floor(frag.xy);
    let g = select(0.05, 0.9, (i32(p.x) % 8 == 0) || (i32(p.y) % 8 == 0));
    return vec4<f32>(vec3<f32>(g) * vec3<f32>(0.6, 0.8, 1.0), 1.0);
}
