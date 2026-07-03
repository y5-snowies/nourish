use compositor_orchestration_draw_dispatch_frame::{NativeShaderPass, ParallaxUniforms, ShaderVariant};
use std::borrow::Cow;

/// Stable per-variant pipeline-cache ids for the renderer's shader-pass map.
const SDR_ID: u64 = 0x7061_7261_0001; // "para" #1 (SDR)
const HDR_ID: u64 = 0x7061_7261_0002; // "para" #2 (HDR)

/// The built-in SDR parallax WGSL source, exposed so the settings live preview
/// can render the built-in shader (the selected user shader supplies its own).
pub const PARALLAX_WGSL: &str = include_str!("shaders/parallax.wgsl");

/// SPIR-V modules (each holds `vs_main` + `fs_main`), naga-compiled at build.
const SDR_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/parallax.spv"));
const HDR_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/parallax_hdr.spv"));

/// SDR push — matches `parallax.wgsl`'s `Push` (engine 3×vec4 + params 4×vec4 =
/// 112 bytes). `params` carries the shader-authored `@prop` values.
#[repr(C)]
#[derive(Clone, Copy)]
struct SdrPush {
    res_zoom_time: [f32; 4],
    pan_flow: [f32; 4],
    lock_alpha: [f32; 4],
    params: [[f32; 4]; 4],
}

/// HDR push — the SDR fields plus the HDR levels (8×vec4 = 128 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct HdrPush {
    res_zoom_time: [f32; 4],
    pan_flow: [f32; 4],
    lock_alpha: [f32; 4],
    params: [[f32; 4]; 4],
    /// x = sdr_white_nits, y = max_nits, z/w reserved.
    hdr: [f32; 4],
}

/// Split the 16-float params block into four `vec4`s (the std140 push layout).
fn params_vec4(p: &[f32; 16]) -> [[f32; 4]; 4] {
    [
        [p[0], p[1], p[2], p[3]],
        [p[4], p[5], p[6], p[7]],
        [p[8], p[9], p[10], p[11]],
        [p[12], p[13], p[14], p[15]],
    ]
}

fn as_bytes<T: Copy>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((v as *const T) as *const u8, std::mem::size_of::<T>()) }
}

/// Owns this frame's packed push payloads so the borrowed `NativeShaderPass`
/// stays valid for the duration of the dispatch call. Build it in the render
/// element's `draw`, then hand `pass()` to `SceneDispatch::draw_pixel_program`.
pub struct ParallaxPass {
    sdr: SdrPush,
    hdr: HdrPush,
}

impl ParallaxPass {
    /// Pack both variants' push constants from the renderer-agnostic uniforms.
    /// The HDR levels come from the live HDR tuning registry (ignored unless the
    /// renderer is compositing HDR, in which case it selects the HDR variant).
    pub fn new(u: &ParallaxUniforms, params: &[f32; 16]) -> Self {
        let res_zoom_time = [u.resolution[0], u.resolution[1], u.zoom, u.time];
        let pan_flow = [u.pan[0], u.pan[1], u.flow_offset[0], u.flow_offset[1]];
        let lock_alpha = [u.lock_amount, u.alpha, 0.0, 0.0];
        let params = params_vec4(params);
        let t = compositor_developer_stats_registry_base::base::hdr_tuning();
        Self {
            sdr: SdrPush {
                res_zoom_time,
                pan_flow,
                lock_alpha,
                params,
            },
            hdr: HdrPush {
                res_zoom_time,
                pan_flow,
                lock_alpha,
                params,
                hdr: [t.sdr_white_nits, t.max_nits, 0.0, 0.0],
            },
        }
    }

    /// The dispatch-seam draw request: the SDR variant + the HDR-output variant.
    pub fn pass(&self) -> NativeShaderPass<'_> {
        NativeShaderPass {
            sdr: ShaderVariant {
                id: SDR_ID,
                spv: Cow::Borrowed(SDR_SPV),
                vert_spv: None,
                vert_entry: Cow::Borrowed("vs_main"),
                frag_entry: Cow::Borrowed("fs_main"),
                push: Cow::Borrowed(as_bytes(&self.sdr)),
            },
            hdr: Some(ShaderVariant {
                id: HDR_ID,
                spv: Cow::Borrowed(HDR_SPV),
                vert_spv: None,
                vert_entry: Cow::Borrowed("vs_main"),
                frag_entry: Cow::Borrowed("fs_main"),
                push: Cow::Borrowed(as_bytes(&self.hdr)),
            }),
        }
    }
}

/// Pack the standard 112-byte engine push (`res_zoom_time` / `pan_flow` /
/// `lock_alpha` + the 4×vec4 `params` block) for a runtime-loaded WGSL/GLSL
/// background shader, which uses the same `Push` layout as `parallax.wgsl`.
pub fn engine_push(u: &ParallaxUniforms, params: &[f32; 16]) -> [u8; 112] {
    let p = SdrPush {
        res_zoom_time: [u.resolution[0], u.resolution[1], u.zoom, u.time],
        pan_flow: [u.pan[0], u.pan[1], u.flow_offset[0], u.flow_offset[1]],
        lock_alpha: [u.lock_amount, u.alpha, 0.0, 0.0],
        params: params_vec4(params),
    };
    let mut out = [0u8; 112];
    out.copy_from_slice(as_bytes(&p));
    out
}
