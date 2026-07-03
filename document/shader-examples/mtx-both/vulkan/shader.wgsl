// MATRIX CELL: BOTH renderers — this file (vulkan/, WGSL→SPIR-V) runs on Vulkan;
// the sibling gles/shader.frag (ES-3 GLSL) runs on GLES. Same posterized look.
// @prop steps float default=6.0 min=2.0 max=16.0 step=1.0 label="Posterize steps"
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;
struct VsOut { @builtin(position) pos: vec4<f32> };
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut; o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0); return o;
}
@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let steps = pc.params[0].x;
    let uv = frag.xy / res;
    let v = sin(uv.x * 6.0 + t) * 0.5 + 0.5;
    let q = round(v * steps) / steps;
    return vec4<f32>(vec3<f32>(0.5, 0.5, 0.9) * q + vec3<f32>(0.03, 0.02, 0.06), 1.0);
}
