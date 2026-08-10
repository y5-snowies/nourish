//! The push-constant layout every pass receives, and the one function that packs
//! it.
//!
//! 112 bytes: three engine `vec4`s (resolution/zoom/time, pan/flow,
//! lock/alpha/sRGB/velocity) followed by four `vec4`s of `@prop` values. Every
//! shipped bundle restates this as its own `struct Push`, and none of them can
//! be recompiled from here, so the layout is an ABI and is written down beside
//! the packer.
//!
//! It lived in `background.two/two.draw/draw.vulkan` because that is where the
//! parallax SPIR-V is built — but the layout is not the parallax's, it is every
//! pass's. Keeping it there made the pipeline's own push ABI a thing the
//! pipeline had to reach into the background to get.

use compositor_orchestration_draw_dispatch_frame::ParallaxUniforms;

/// The full-scale pan velocity (world px/s) of the packed push lane — one lane
/// carries both components, so shaders decode with
/// `unpack2x16snorm(bitcast<u32>(lock_alpha.w)) * VELOCITY_LANE_SCALE`.
pub const VELOCITY_LANE_SCALE: f32 = 16384.0;

/// SDR push — matches `parallax.wgsl`'s `Push` (engine 3×vec4 + params 4×vec4 =
/// 112 bytes). `params` carries the shader-authored `@prop` values.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SdrPush {
    pub res_zoom_time: [f32; 4],
    pub pan_flow: [f32; 4],
    pub lock_alpha: [f32; 4],
    pub params: [[f32; 4]; 4],
}

/// Split the 16-float params block into four `vec4`s (the std140 push layout).
pub fn params_vec4(p: &[f32; 16]) -> [[f32; 4]; 4] {
    [
        [p[0], p[1], p[2], p[3]],
        [p[4], p[5], p[6], p[7]],
        [p[8], p[9], p[10], p[11]],
        [p[12], p[13], p[14], p[15]],
    ]
}

pub fn as_bytes<T: Copy>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((v as *const T) as *const u8, std::mem::size_of::<T>()) }
}

/// Pack the smoothed pan velocity into one push lane as two snorm16 halves.
/// One lane suffices because velocity only drives visual stretch — quantisation
/// (~0.5 px/s) is far below anything visible. Lane z stays free for the sRGB
/// flag. snorm (not f16) so the preview's capability-less naga accepts the
/// decode.
pub fn velocity_lane(v: [f32; 2]) -> f32 {
    let q = |x: f32| -> u32 {
        let n = (x / VELOCITY_LANE_SCALE).clamp(-1.0, 1.0);
        ((n * 32767.0).round() as i32 as u32) & 0xffff
    };
    f32::from_bits(q(v[0]) | (q(v[1]) << 16))
}

/// Pack the standard 112-byte engine push (`res_zoom_time` / `pan_flow` /
/// `lock_alpha` + the 4×vec4 `params` block) for a runtime-loaded WGSL/GLSL
/// background shader, which uses the same `Push` layout as `parallax.wgsl`.
pub fn engine_push(u: &ParallaxUniforms, params: &[f32; 16]) -> [u8; 112] {
    let p = SdrPush {
        res_zoom_time: [u.resolution[0], u.resolution[1], u.zoom, u.time],
        pan_flow: [u.pan[0], u.pan[1], u.flow_offset[0], u.flow_offset[1]],
        lock_alpha: [u.lock_amount, u.alpha, u.srgb, velocity_lane(u.velocity)],
        params: params_vec4(params),
    };
    let mut out = [0u8; 112];
    out.copy_from_slice(as_bytes(&p));
    out
}
