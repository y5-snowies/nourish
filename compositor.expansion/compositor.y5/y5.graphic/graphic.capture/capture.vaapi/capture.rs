//! Backend selection, the capture settings accessors, and the running-encode
//! handle. Re-exported at the crate root.
use crate::{Codec, EncoderThread, NvencCudaEncoder, VaapiEncoder};
use std::path::PathBuf;

/// Which hardware video backend to use. Selected by the `capture_encoder`
/// settings field (`mesa`/`vaapi` → VAAPI, anything else → NVENC). NVENC is the
/// path under test (NVIDIA has no VAAPI encode).
pub enum Backend {
    Nvenc,
    Vaapi,
}

pub fn backend_from_config() -> Backend {
    let encoder = &compositor_model_environment_config_base::base::get().capture_encoder;
    match encoder.as_str() {
        "mesa" | "vaapi" => Backend::Vaapi,
        _ => Backend::Nvenc,
    }
}

/// The DRM render node the capture/encode pipeline targets (`render_node`
/// config, e.g. `/dev/dri/renderD128`). Used for the VAAPI encode device and for
/// VAAPI hardware *decode* in the background re-encode.
pub fn capture_render_node() -> String {
    compositor_model_environment_config_base::base::get()
        .render_node
        .clone()
}

/// Live capture frame rate (`capture_refresh_rate_max`, clamped to 30..=120).
pub fn capture_fps() -> u32 {
    compositor_model_environment_config_base::base::get()
        .capture_refresh_rate_max
        .clamp(30, 120)
}

/// Live NVENC constant-quality value from the `capture_quality` preset:
/// `lossless` → 19 (near-lossless), `optimized` → 28 (smaller, still real-time).
pub fn capture_cq() -> u32 {
    match compositor_model_environment_config_base::base::get()
        .capture_quality
        .as_str()
    {
        "optimized" => 28,
        _ => 19,
    }
}

/// Whether a failed NVENC zero-copy start may fall back to the readback encoder
/// (`capture_nvenc_allow_readback_fallback`). Default false → abort with an error
/// dialog instead.
pub fn capture_nvenc_readback_fallback() -> bool {
    compositor_model_environment_config_base::base::get().capture_nvenc_allow_readback_fallback
}

/// Whether the optimized software re-encode runs automatically in the background
/// after every recording (`capture_background_encoder == "ffmpeg"`). When false,
/// the save dialog offers it as a checkbox instead.
pub fn capture_background_auto() -> bool {
    compositor_model_environment_config_base::base::get().capture_background_encoder == "ffmpeg"
}

/// Whether the saved file must be constant-frame-rate (`capture_variable_frame_rate
/// == false`). CFR is produced by the re-encode pass; when true the driver forces
/// a re-encode even for an otherwise plain save.
pub fn capture_cfr() -> bool {
    !compositor_model_environment_config_base::base::get().capture_variable_frame_rate
}

/// Normalized `capture_codec` string (`av1`|`h265`|`h264`, default `av1`). The
/// driver maps this to the software re-encode codec.
pub fn capture_codec_name() -> &'static str {
    match compositor_model_environment_config_base::base::get()
        .capture_codec
        .as_str()
    {
        "h264" => "h264",
        "h265" => "h265",
        _ => "av1",
    }
}

/// The NVENC codec fallback list for the `capture_codec` preference. The driver
/// tries each in order until one's encoder opens (av1 needs Ada; h265/h264 are
/// broad). VP9 has no NVENC, so it's absent here (software re-encode only).
pub fn capture_codecs() -> Vec<Codec> {
    match compositor_model_environment_config_base::base::get()
        .capture_codec
        .as_str()
    {
        "av1" => vec![Codec::Av1, Codec::Hevc, Codec::H264],
        "h265" => vec![Codec::Hevc, Codec::H264],
        _ => vec![Codec::H264],
    }
}

/// A running video encode session:
/// - `NvencCuda` — zero-copy dmabuf → EGL → CUDA → NVENC (preferred on NVIDIA);
/// - `Nvenc` — NVENC fed by GPU→CPU readback, encoded on its own [`EncoderThread`]
///   (fallback when the zero-copy path is unavailable);
/// - `Vaapi` — zero-copy dmabuf, encoded inline.
pub enum CaptureEncoder {
    Nvenc(EncoderThread),
    NvencCuda(NvencCudaEncoder),
    Vaapi(VaapiEncoder),
}

impl CaptureEncoder {
    /// Only the readback NVENC variant needs the GPU→CPU readback frames; the
    /// zero-copy variants (`NvencCuda`, `Vaapi`) read the dmabuf directly.
    pub fn needs_readback(&self) -> bool {
        matches!(self, Self::Nvenc(_))
    }

    pub fn finish(self) -> Option<PathBuf> {
        match self {
            Self::Nvenc(n) => n.finish(),
            Self::NvencCuda(z) => z.finish(),
            Self::Vaapi(v) => v.finish(),
        }
    }

    pub fn discard(self) {
        match self {
            Self::Nvenc(n) => n.discard(),
            Self::NvencCuda(z) => z.discard(),
            Self::Vaapi(v) => v.discard(),
        }
    }
}
