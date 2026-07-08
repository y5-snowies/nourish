//! Fedora-only RPM Fusion enablement + the full-FFmpeg swap. Kept apart from the generic
//! `enumerate.install` runner so the Fedora-isms don't leak into the apt/pacman paths.
//! Only ever called on the `Dnf` path (execute.packages). Pure std.

use compositor_installer_process_packages_enumerate_install::run_sudo;
use std::process::Command;

/// Enable the RPM Fusion **free** repo (its `-release` rpm), so `mesa-va-drivers-freeworld`
/// (hardware VA-API video) becomes installable. Opt-in only.
pub fn enable_rpmfusion_free(dry_run: bool) -> Result<(), String> {
    enable_rpmfusion("free", dry_run)
}

/// Enable the RPM Fusion **nonfree** repo, so `intel-media-driver` (the iHD VA-API driver
/// for Gen8+ Intel iGPUs, e.g. Kaby Lake) becomes installable. The nonfree `-release` rpm
/// depends on the free one, so enable free first. Opt-in only.
pub fn enable_rpmfusion_nonfree(dry_run: bool) -> Result<(), String> {
    enable_rpmfusion("nonfree", dry_run)
}

/// Swap Fedora's codec-stripped `ffmpeg-free` for RPM Fusion's full `ffmpeg`
/// (`--allowerasing`, since the full libs replace the `-free` ones). Needed for screen
/// capture on every machine — NVENC and VAAPI both encode through FFmpeg. Needs RPM
/// Fusion (free) enabled first. Opt-in.
pub fn swap_ffmpeg_full(dry_run: bool) -> Result<(), String> {
    run_sudo(
        &["dnf".into(), "swap".into(), "-y".into(), "ffmpeg-free".into(), "ffmpeg".into(), "--allowerasing".into()],
        dry_run,
    )
}

/// Install an RPM Fusion `<kind>-release` rpm (`kind` = "free" | "nonfree").
fn enable_rpmfusion(kind: &str, dry_run: bool) -> Result<(), String> {
    let rel = fedora_release();
    let url = format!(
        "https://mirrors.rpmfusion.org/{kind}/fedora/rpmfusion-{kind}-release-{rel}.noarch.rpm"
    );
    println!("Enabling RPM Fusion ({kind}) for Fedora {rel}...");
    run_sudo(&["dnf".into(), "install".into(), "-y".into(), url], dry_run)
}

/// `rpm -E %fedora` — the running Fedora release number; falls back to the bundle's
/// target (44) if rpm can't be queried.
fn fedora_release() -> String {
    Command::new("rpm")
        .args(["-E", "%fedora"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_else(|| "44".to_string())
}
