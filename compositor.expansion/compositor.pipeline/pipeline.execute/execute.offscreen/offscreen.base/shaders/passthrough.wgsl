// Phase-0 passthrough: sample the offscreen `content` image and write it to the
// swapchain unchanged. `content` and the swapchain share extent+format, and the
// sample coordinate hits texel centers (uv = (px+0.5)/res), so bilinear returns
// the exact texel — the composited frame is reproduced byte-for-byte. The whole
// offscreen path exists so later passes can run AFTER windows and SAMPLE the
// composited scene (see document/SHADER_PIPELINE.md); on its own it is a no-op.

struct Push { res: vec4<f32> };   // xy = swapchain pixel size
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

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
    let uv = frag.xy / pc.res.xy;
    return textureSample(src, samp, uv);
}
