use compositor_orchestration_draw_dispatch_frame::{NativeShaderPass, ParallaxUniforms, ShaderVariant};
use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

/// Stable per-variant pipeline-cache ids for the renderer's shader-pass map.
const SDR_ID: u64 = 0x7061_7261_0001; // "para" #1 (SDR)
const HDR_ID: u64 = 0x7061_7261_0002; // "para" #2 (HDR)
const OPT_ID: u64 = 0x7061_7261_0003; // "para" #3 (SDR, optimized)

/// The built-in SDR parallax WGSL source, exposed so the settings live preview
/// can render the built-in shader (the selected user shader supplies its own).
pub const PARALLAX_WGSL: &str = include_str!("shaders/parallax.wgsl");

/// The optimized SDR variant's source, for the same preview when the per-world
/// "Optimized" toggle is on — so the panel shows what is actually on screen.
pub const PARALLAX_OPTIMIZED_WGSL: &str = include_str!("shaders/parallax_optimized.wgsl");

/// SPIR-V modules (each holds `vs_main` + `fs_main`), naga-compiled at build.
const SDR_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/parallax.spv"));
const HDR_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/parallax_hdr.spv"));
const OPT_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/parallax_optimized.spv"));

/// The built-ins as the `Arc`s the draw seam carries, allocated once on first
/// use. The bytes are `'static`, but the seam is `Arc`-based so a runtime-loaded
/// shader can cross it without a per-draw copy; paying one copy here at startup
/// keeps the built-in path on the same cheap refcount-bump hand-off.
struct Builtin {
    sdr: Arc<[u8]>,
    hdr: Arc<[u8]>,
    opt: Arc<[u8]>,
    vs: Arc<str>,
    fs: Arc<str>,
}

fn builtin() -> &'static Builtin {
    static B: OnceLock<Builtin> = OnceLock::new();
    B.get_or_init(|| Builtin {
        sdr: Arc::from(SDR_SPV),
        hdr: Arc::from(HDR_SPV),
        opt: Arc::from(OPT_SPV),
        vs: Arc::from("vs_main"),
        fs: Arc::from("fs_main"),
    })
}

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
    /// Per-world "Optimized" toggle: fill the SDR slot with the cheap variant.
    /// Producer-side, unlike the renderer-chosen SDR/HDR split — so it swaps the
    /// SPIR-V behind the existing slot rather than adding a third one.
    optimized: bool,
}

/// The full-scale pan velocity (world px/s) of the packed push lane — one lane
/// carries both components, so shaders decode with
/// `unpack2x16snorm(bitcast<u32>(lock_alpha.w)) * VELOCITY_LANE_SCALE`.
pub const VELOCITY_LANE_SCALE: f32 = 16384.0;

/// Pack the smoothed pan velocity into one push lane as two snorm16 halves.
/// One lane suffices because velocity only drives visual stretch — quantisation
/// (~0.5 px/s) is far below anything visible. Lane z stays free for the sRGB
/// flag. snorm (not f16) so the preview's capability-less naga accepts the
/// decode.
fn velocity_lane(v: [f32; 2]) -> f32 {
    let q = |x: f32| -> u32 {
        let n = (x / VELOCITY_LANE_SCALE).clamp(-1.0, 1.0);
        ((n * 32767.0).round() as i32 as u32) & 0xffff
    };
    f32::from_bits(q(v[0]) | (q(v[1]) << 16))
}

impl ParallaxPass {
    /// Pack both variants' push constants from the renderer-agnostic uniforms.
    /// The HDR levels come from the live HDR tuning registry (ignored unless the
    /// renderer is compositing HDR, in which case it selects the HDR variant).
    pub fn new(u: &ParallaxUniforms, params: &[f32; 16], optimized: bool) -> Self {
        let res_zoom_time = [u.resolution[0], u.resolution[1], u.zoom, u.time];
        let pan_flow = [u.pan[0], u.pan[1], u.flow_offset[0], u.flow_offset[1]];
        // z = per-world sRGB flag; w = the smoothed pan velocity for
        // velocity-reactive shaders, packed as two f16 halves (unpack2x16float).
        let lock_alpha = [u.lock_amount, u.alpha, u.srgb, velocity_lane(u.velocity)];
        let params = params_vec4(params);
        let t = compositor_model_stats_registry_base::base::hdr_tuning();
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
            optimized,
        }
    }

    /// The dispatch-seam draw request: the SDR variant + the HDR-output variant.
    ///
    /// "Optimized" swaps which SPIR-V fills the SDR slot. The HDR variant is
    /// deliberately left alone — there is no optimized HDR twin, and the machines
    /// this toggle exists for do not drive HDR output. Because the ids differ the
    /// renderer's `(id, format)` pipeline cache holds both, so toggling costs one
    /// pipeline build the first time and nothing after.
    pub fn pass(&self) -> NativeShaderPass<'_> {
        let b = builtin();
        let (sdr_id, sdr_spv) = if self.optimized {
            (OPT_ID, &b.opt)
        } else {
            (SDR_ID, &b.sdr)
        };
        NativeShaderPass {
            sdr: ShaderVariant {
                id: sdr_id,
                spv: Arc::clone(sdr_spv),
                vert_spv: None,
                vert_entry: Arc::clone(&b.vs),
                frag_entry: Arc::clone(&b.fs),
                push: Cow::Borrowed(as_bytes(&self.sdr)),
            },
            hdr: Some(ShaderVariant {
                id: HDR_ID,
                spv: Arc::clone(&b.hdr),
                vert_spv: None,
                vert_entry: Arc::clone(&b.vs),
                frag_entry: Arc::clone(&b.fs),
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
        lock_alpha: [u.lock_amount, u.alpha, u.srgb, velocity_lane(u.velocity)],
        params: params_vec4(params),
    };
    let mut out = [0u8; 112];
    out.copy_from_slice(as_bytes(&p));
    out
}
