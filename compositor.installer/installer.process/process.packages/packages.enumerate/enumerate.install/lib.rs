//! The system package-install runner, generic over the detected package manager
//! (`dnf` / `apt-get` / `pacman`), plus the Debian `bookworm-backports` enabler. The
//! Fedora-only RPM Fusion steps live in the sibling `enumerate.rpmfusion` crate (which
//! reuses `run_sudo` from here). Pure std.

use compositor_installer_process_packages_enumerate_platform::PackageManager;
use std::process::Command;

/// Install `packages` with the detected manager. With `dry_run`, commands are printed only.
///
/// Strict: a non-zero exit (including an unavailable package) is returned as an error so
/// the caller ABORTS rather than continuing with a half-installed system. `Nix` is never
/// installed transactionally (it's declarative) — execute.packages handles it separately
/// and never calls this with `Nix`; guarded here defensively.
pub fn pkg_install(mgr: PackageManager, packages: &[String], dry_run: bool) -> Result<(), String> {
    if packages.is_empty() {
        return Ok(());
    }
    match mgr {
        PackageManager::Dnf => {
            let mut argv: Vec<String> = vec!["dnf".into(), "install".into(), "-y".into()];
            argv.extend(packages.iter().cloned());
            run_sudo(&argv, dry_run)
        }
        PackageManager::Apt => {
            // Refresh indexes first (a fresh container / long-idle box may have stale
            // lists), then a non-recommends install so we don't drag in extras.
            run_sudo(&["apt-get".into(), "update".into()], dry_run)?;
            let mut argv: Vec<String> = vec![
                "apt-get".into(), "install".into(), "-y".into(), "--no-install-recommends".into(),
            ];
            argv.extend(packages.iter().cloned());
            run_sudo(&argv, dry_run)
        }
        PackageManager::Pacman => {
            let mut argv: Vec<String> =
                vec!["pacman".into(), "-S".into(), "--needed".into(), "--noconfirm".into()];
            argv.extend(packages.iter().cloned());
            run_sudo(&argv, dry_run)
        }
        PackageManager::Nix => Err(
            "internal: pkg_install called for NixOS — the Nix path prints a profile instead".into(),
        ),
    }
}

/// Enable Debian `bookworm-backports` (where `libdisplay-info2` lives) and refresh the
/// index. Debian 12 only — trixie/noble carry the lib in main/universe. Mirrors the
/// debian-12 Containerfile's backports line.
pub fn apt_enable_backports(dry_run: bool) -> Result<(), String> {
    const LINE: &str = "deb http://deb.debian.org/debian bookworm-backports main";
    let write = format!("echo '{LINE}' > /etc/apt/sources.list.d/backports.list");
    run_sudo(&["bash".into(), "-c".into(), write], dry_run)?;
    run_sudo(&["apt-get".into(), "update".into()], dry_run)
}

/// Run `sudo <argv>`, or just print it under `dry_run`. Non-zero exit -> Err. Public so
/// `enumerate.rpmfusion` can drive the Fedora-specific `dnf` steps through the same path.
pub fn run_sudo(argv: &[String], dry_run: bool) -> Result<(), String> {
    if dry_run {
        println!("  [dry-run] sudo {}", argv.join(" "));
        return Ok(());
    }
    let status = Command::new("sudo")
        .args(argv)
        .status()
        .map_err(|e| format!("failed to run sudo {}: {e}", argv.first().map_or("", |s| s)))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{}` exited with {status}", argv.join(" ")))
    }
}
