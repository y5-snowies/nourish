use std::process::Command;

/// A user-selectable group of dnf packages.
#[derive(Clone, Debug)]
pub struct PackageGroup {
    /// Stable key, e.g. "base", "mesa", "nvidia".
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub packages: Vec<&'static str>,
    /// Whether this group is pre-selected by default.
    pub default_on: bool,
}

/// Detected primary GPU vendor (best-effort, from `lspci`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

/// Best-effort GPU vendor detection from `lspci`. Used to pre-select the Mesa vs
/// NVIDIA driver group; never fatal.
pub fn detect_gpu() -> Gpu {
    let out = Command::new("lspci").output();
    let text = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_lowercase(),
        Err(_) => return Gpu::Unknown,
    };
    // Only consider VGA / 3D / Display controller lines.
    let gpu_lines: String = text
        .lines()
        .filter(|l| l.contains("vga") || l.contains("3d controller") || l.contains("display controller"))
        .collect::<Vec<_>>()
        .join("\n");
    // Intel is checked before AMD on purpose: every PCI line says "<vendor> Corporation",
    // and a bare "ati" substring matches "corpor-ati-on" — so an Intel iGPU ("Intel
    // Corporation …") would otherwise be misread as AMD. Intel never says "amd"/"radeon".
    if gpu_lines.contains("nvidia") {
        Gpu::Nvidia
    } else if gpu_lines.contains("intel") {
        Gpu::Intel
    } else if gpu_lines.contains("amd") || gpu_lines.contains("radeon") || gpu_lines.contains("ati ") {
        Gpu::Amd
    } else {
        Gpu::Unknown
    }
}

/// The default `capture_encoder` for the detected GPU: NVIDIA uses NVENC, everything
/// else uses VAAPI (Mesa, the only HW video-encode path on AMD/Intel). Unknown falls
/// back to VAAPI — the broadly-compatible choice when there's no NVIDIA present.
pub fn capture_encoder_for(gpu: Gpu) -> &'static str {
    match gpu {
        Gpu::Nvidia => "nvenc",
        _ => "vaapi",
    }
}

/// Which kernel driver is bound to an NVIDIA GPU. Nourish ships NO driver, so this is
/// reported (to warn the user) — never acted on. Only meaningful with an NVIDIA GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvidiaDriver {
    /// The proprietary `nvidia` kernel driver is bound — Vulkan/CUDA acceleration works.
    Proprietary,
    /// The open-source `nouveau` driver is bound — unsupported for Nourish.
    Nouveau,
    /// No driver bound, or undetectable — the proprietary stack is missing.
    Missing,
}

/// Read the kernel driver actually bound to the NVIDIA GPU from `lspci -k`. Walks the
/// NVIDIA VGA/3D/display block and reads its "Kernel driver in use:" line; best-effort.
pub fn nvidia_driver_status() -> NvidiaDriver {
    let out = Command::new("lspci").arg("-k").output();
    let text = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_lowercase(),
        Err(_) => return NvidiaDriver::Missing,
    };
    let mut in_nvidia_gpu = false;
    for line in text.lines() {
        // Device-header lines start at column 0; their indented detail lines don't.
        if !line.starts_with([' ', '\t']) {
            in_nvidia_gpu = line.contains("nvidia")
                && (line.contains("vga") || line.contains("3d controller") || line.contains("display controller"));
        } else if in_nvidia_gpu && line.trim_start().starts_with("kernel driver in use:") {
            if line.contains("nvidia") {
                return NvidiaDriver::Proprietary;
            } else if line.contains("nouveau") {
                return NvidiaDriver::Nouveau;
            }
        }
    }
    NvidiaDriver::Missing
}
