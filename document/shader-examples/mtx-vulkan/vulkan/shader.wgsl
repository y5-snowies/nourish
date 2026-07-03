// MATRIX CELL: `vulkan/` format on the VULKAN renderer (WGSL → SPIR-V via naga).
// Feature exercised: native WGSL (var<immediate> push block, array<vec4>, WGSL syntax).
// @prop speed float default=1.0 min=0.0 max=4.0 step=0.05 label="Speed"
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
    let speed = pc.params[0].x;
    let uv = frag.xy / res;
    let bands = sin(uv.x * 18.0 + t * speed) * 0.5 + 0.5;
    return vec4<f32>(vec3<f32>(0.85, 0.30, 0.55) * bands + vec3<f32>(0.02, 0.02, 0.06), 1.0);
}
