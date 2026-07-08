//! Installer step 1: detect the distro's package manager + GPU, prompt the package
//! groups, install the selection with that manager, and wire up hardware VA-API per
//! platform. NixOS is special-cased: it prints a `configuration.nix` (nix-ld) snippet to
//! add instead of installing. On NVIDIA we only check the bound driver and warn (Nourish
//! never installs the proprietary NVIDIA driver).

use compositor_installer_process_config_parse_base::prompt;
use compositor_installer_process_packages_enumerate_base as pkg;
use compositor_installer_process_packages_enumerate_base::{Gpu, NvidiaDriver, PackageGroup, PackageManager};

/// Mesa Gallium VAAPI driver (RPM Fusion free) — Fedora name.
const MESA_VA_FREEWORLD: &str = "mesa-va-drivers-freeworld";
/// Intel iHD VA-API driver (RPM Fusion nonfree) — for Gen8+ Intel iGPUs (e.g. Kaby Lake).
const INTEL_MEDIA_DRIVER: &str = "intel-media-driver";
/// Fedora's codec-stripped FFmpeg runtime libs carried by the `runtime` group. When the
/// user opts into full FFmpeg these are dropped from the base install: the swap below
/// pulls the full libav stack and erases Fedora's preinstalled `-free` packages in ONE
/// pass, so installing them here first would only get them immediately erased again.
const FFMPEG_FREE_LIBS: &[&str] = &[
    "libavutil-free", "libavcodec-free", "libavformat-free", "libavfilter-free", "libswscale-free",
];

/// Detect the package manager + GPU, prompt every package group, then install the
/// resolved selection with the right package manager (or, on NixOS, print the profile to
/// add). STRICT for the transactional managers: a real install failure aborts.
pub fn select_and_install(dry_run: bool) -> Result<(), String> {
    let mgr = PackageManager::detect();
    let release = pkg::release_id();
    let gpu = pkg::detect_gpu();
    println!("\nDetected package manager: {} · GPU vendor: {gpu:?}", mgr.command());

    // 1) Package groups (same prompt UX across managers).
    println!("\n-- Package groups (Enter keeps the [default]) --");
    let selected: Vec<PackageGroup> = pkg::groups(mgr, release.as_deref())
        .into_iter()
        .filter(|g| prompt::yes_no(g.key, &format!("{} — {}", g.title, g.description), g.default_on))
        .collect();

    // 2) NixOS is declarative + non-FHS: nothing to install — print the nix-ld profile.
    if mgr == PackageManager::Nix {
        return print_nix_profile(&selected);
    }

    let packages: Vec<String> =
        selected.iter().flat_map(|g| g.packages.iter().map(|s| s.to_string())).collect();

    // 3) Manager-specific install + hardware VA-API.
    match mgr {
        PackageManager::Dnf => install_fedora(packages, gpu, dry_run)?,
        PackageManager::Apt => install_apt(packages, release.as_deref(), gpu, dry_run)?,
        PackageManager::Pacman => install_pacman(packages, gpu, dry_run)?,
        PackageManager::Nix => unreachable!("handled above"),
    }

    // Nourish ships no NVIDIA driver; if the hardware is here, just report its state.
    if gpu == Gpu::Nvidia {
        warn_nvidia_driver(pkg::nvidia_driver_status());
    }
    Ok(())
}

/// Fedora path: prompt the RPM-Fusion capture/VA-API options up front (so the base `dnf`
/// already targets the final set), install, then apply the RPM-Fusion choices. Unchanged
/// from the original Fedora-only installer.
fn install_fedora(mut packages: Vec<String>, gpu: Gpu, dry_run: bool) -> Result<(), String> {
    println!("\n-- Screen capture / VA-API (RPM Fusion) --");
    let intel = gpu == Gpu::Intel;
    let want_ffmpeg = prompt::yes_no("Full FFmpeg (RPM Fusion)", "Swap Fedora's codec-stripped ffmpeg-free for the full ffmpeg — REQUIRED for screen capture on every machine (NVENC + VAAPI both encode through it). Enables RPM Fusion (free).", true);
    let want_mesa_va = prompt::yes_no("mesa-va-drivers-freeworld (RPM Fusion)", "Mesa VAAPI driver — REQUIRED by the compositor's Vulkan renderer + VA-API capture. Enables RPM Fusion (free). Recommended.", true);
    let want_intel_media = prompt::yes_no("intel-media-driver (RPM Fusion)", "Intel iHD VA-API driver — Gen8+ Intel iGPUs (e.g. Kaby Lake) need it for the Vulkan renderer + capture. Enables RPM Fusion (nonfree). Recommended on Intel only.", intel);

    // When swapping to full FFmpeg, drop the `-free` libs from the base install so they
    // are never installed only to be erased by the swap (no double-install of ffmpeg).
    if want_ffmpeg {
        packages.retain(|p| !FFMPEG_FREE_LIBS.contains(&p.as_str()));
    }

    base_install(PackageManager::Dnf, &packages, dry_run)?;

    if want_ffmpeg {
        pkg::enable_rpmfusion_free(dry_run).map_err(|e| format!("RPM Fusion (free) failed: {e}"))?;
        pkg::swap_ffmpeg_full(dry_run).map_err(|e| format!("ffmpeg swap failed: {e}"))?;
    }
    if want_mesa_va {
        install_va_rpmfusion(&[MESA_VA_FREEWORLD.to_string()], false, dry_run)?;
    }
    if want_intel_media {
        install_va_rpmfusion(&[INTEL_MEDIA_DRIVER.to_string()], true, dry_run)?;
    }
    Ok(())
}

/// Debian/Ubuntu path: enable bookworm-backports where the EDID lib lives (Debian 12
/// only), fold the native VA-API drivers into the transaction (ffmpeg is already complete
/// in the runtime libs — no repo dance), then one apt install.
fn install_apt(mut packages: Vec<String>, release: Option<&str>, gpu: Gpu, dry_run: bool) -> Result<(), String> {
    use compositor_installer_process_packages_enumerate_debian::needs_backports;
    if needs_backports(release) {
        pkg::apt_enable_backports(dry_run).map_err(|e| format!("enabling bookworm-backports failed: {e}"))?;
    }
    // Native VA-API drivers (no extra repos): generic Mesa + Intel's iHD on Intel iGPUs.
    packages.push("mesa-va-drivers".into());
    if gpu == Gpu::Intel {
        packages.push("intel-media-va-driver".into());
    }
    base_install(PackageManager::Apt, &packages, dry_run)
}

/// Arch path: add the vendor Vulkan ICD for the detected GPU (Arch has no generic
/// mesa-vulkan-drivers) + the native VA-API drivers, then one pacman transaction. ffmpeg
/// is complete in the runtime group already.
fn install_pacman(mut packages: Vec<String>, gpu: Gpu, dry_run: bool) -> Result<(), String> {
    match gpu {
        Gpu::Amd => packages.push("vulkan-radeon".into()),
        Gpu::Intel => packages.extend(["vulkan-intel".into(), "intel-media-driver".into()]),
        _ => {}
    }
    packages.push("libva-mesa-driver".into());
    base_install(PackageManager::Pacman, &packages, dry_run)
}

/// Run the base transaction, printing a count first (shared by every manager).
fn base_install(mgr: PackageManager, packages: &[String], dry_run: bool) -> Result<(), String> {
    if packages.is_empty() {
        println!("No package groups selected — skipping {}.", mgr.command());
        return Ok(());
    }
    println!("\nInstalling {} packages via {}...", packages.len(), mgr.command());
    pkg::pkg_install(mgr, packages, dry_run).map_err(|e| format!("package install failed: {e}"))
}

/// Enable RPM Fusion (free, plus nonfree when `nonfree`) and install `pkgs`. Fedora only.
fn install_va_rpmfusion(pkgs: &[String], nonfree: bool, dry_run: bool) -> Result<(), String> {
    pkg::enable_rpmfusion_free(dry_run).map_err(|e| format!("RPM Fusion (free) failed: {e}"))?;
    if nonfree {
        pkg::enable_rpmfusion_nonfree(dry_run).map_err(|e| format!("RPM Fusion (nonfree) failed: {e}"))?;
    }
    pkg::pkg_install(PackageManager::Dnf, pkgs, dry_run)
        .map_err(|e| format!("VA-API driver install failed: {e}"))
}

/// NixOS: print the `configuration.nix` snippet (nix-ld + the selected runtime libs) and
/// how to apply it. There is no transactional install — the prebuilt binaries run under
/// nix-ld, which exposes the listed libraries to them.
fn print_nix_profile(selected: &[PackageGroup]) -> Result<(), String> {
    const C: &str = "\x1b[1;36m";
    const R: &str = "\x1b[0m";
    let snippet = pkg::render_profile(selected);
    println!("\n{C}NixOS detected — no packages are installed directly.{R}");
    println!(
        "The prebuilt y5 binaries are FHS/glibc-linked, so on NixOS they run via `nix-ld`.\n\
         Add the module below to your system configuration, then rebuild:\n"
    );
    println!("── save as e.g. /etc/nixos/y5.nix, then `imports = [ ./y5.nix ];` ──\n");
    println!("{snippet}");
    println!(
        "Apply it:\n  {C}sudo nixos-rebuild switch{R}\n\n\
         Then log out and pick the \"Y5 Compositor\" session. If launching still reports a\n\
         missing library (e.g. `libfoo.so.N: cannot open shared object file`), add that\n\
         library's nixpkgs package to `programs.nix-ld.libraries` and rebuild again."
    );
    Ok(())
}

/// Print a prominent warning when an NVIDIA GPU is present without the proprietary
/// driver bound. No-op when the proprietary stack is already active.
fn warn_nvidia_driver(status: NvidiaDriver) {
    let reason = match status {
        NvidiaDriver::Proprietary => return,
        NvidiaDriver::Nouveau => "the open-source 'nouveau' driver is bound to it",
        NvidiaDriver::Missing => "no NVIDIA kernel driver is bound (the proprietary stack is missing)",
    };
    // Bold-yellow full-width rule banner (no right border, so the double-width ⚠ can't break it).
    const Y: &str = "\x1b[1;33m";
    const R: &str = "\x1b[0m";
    const RULE: &str = "════════════════════════════════════════════════════════════════════════";
    eprintln!("\n{Y}{RULE}{R}");
    eprintln!("{Y}  ⚠  NVIDIA GPU detected — the proprietary driver is NOT active{R}");
    eprintln!("{Y}{RULE}{R}");
    eprintln!("An NVIDIA GPU is present, but {reason}. Nourish renders on Vulkan and does NOT");
    eprintln!("install the NVIDIA driver for you — on nouveau it's slow/unstable. Install it");
    eprintln!("yourself (your distro's proprietary NVIDIA package), reboot, then re-run.");
    eprintln!("Verify with `nvidia-smi`.");
    eprintln!("{Y}────────────────────────────────────────────────────────────────────────{R}");
}
