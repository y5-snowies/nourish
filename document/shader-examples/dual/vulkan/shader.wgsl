// Dual-source bundle (Vulkan half). Explicit per-backend authoring: this WGSL
// runs on Vulkan, the sibling gles/shader.frag runs on GLES — each hand-tuned
// for its renderer. Standard 48-byte engine Push.
//
// @prop warp float default=0.20 min=0.0 max=1.0 step=0.01 label="Warp amount" group="Grid"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>, // @prop slot 0 = warp
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
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let warp = pc.params[0].x; // @prop warp
    var uv = (frag.xy / res - 0.5) * vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    uv = uv + warp * vec2<f32>(sin(uv.y * 4.0 + t), cos(uv.x * 4.0 + t));
    let g = abs(fract(uv * 8.0) - 0.5);
    let line = smoothstep(0.06, 0.0, min(g.x, g.y));
    let col = mix(vec3<f32>(0.03, 0.02, 0.06), vec3<f32>(0.2, 0.5, 0.9), line);
    return vec4<f32>(col, 1.0);
}
