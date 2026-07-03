// Aurora — single-source WGSL bundle. On Vulkan it cross-compiles to SPIR-V and
// renders; on the GLES backend (no gles/ folder here) the loader falls back to
// the built-in parallax. Uses the standard 48-byte engine Push.
//
// @prop speed float default=0.30 min=0.0 max=2.0 step=0.01 label="Drift speed" group="Aurora"
// @prop hue   float default=0.55 min=0.0 max=1.0           label="Hue"          group="Aurora"

struct Push {
    res_zoom_time: vec4<f32>,    // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,         // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,       // x = lock_amount, y = alpha
    params: array<vec4<f32>, 2>, // @prop slots: 0 = speed, 1 = hue
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
    let speed = pc.params[0].x; // @prop speed
    let hue = pc.params[0].y;   // @prop hue
    var uv = frag.xy / res;
    uv.x = uv.x * (res.x / max(res.y, 1.0));
    let wave = sin(uv.x * 6.0 + t * speed) * 0.15 + sin(uv.x * 13.0 - t * speed * 0.7) * 0.05;
    let band = smoothstep(0.0, 0.5, 0.45 - abs(uv.y - 0.55 - wave) * 2.5);
    let base = vec3<f32>(0.02, 0.04, 0.09);
    let tint = mix(vec3<f32>(0.10, 0.85, 0.55), vec3<f32>(0.55, 0.25, 0.85), clamp(hue, 0.0, 1.0));
    let aurora = tint * band;
    let glow = vec3<f32>(0.20, 0.30, 0.55) * smoothstep(1.0, 0.0, uv.y) * 0.4;
    return vec4<f32>(base + aurora + glow, 1.0);
}
