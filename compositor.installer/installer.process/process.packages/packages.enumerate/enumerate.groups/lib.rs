//! Package-group dispatcher: pick the per-distro runtime table for the detected package
//! manager. The group *structure* (keys `runtime`/`xwayland`/`devtool`/`diagnostics`/
//! `toolchain`, titles, defaults) is identical across distros — only the package **names**
//! differ, so each manager gets its own sibling table crate (`enumerate.fedora` /
//! `enumerate.debian` / `enumerate.arch` / `enumerate.nixos`). Pure std.

use compositor_installer_process_packages_enumerate_model::PackageGroup;
use compositor_installer_process_packages_enumerate_platform::PackageManager;

/// Runtime package groups for `mgr`, in that manager's package names. `release` is the
/// `os-release` `VERSION_ID` (only the Debian table uses it, to pick a soversion-suffixed
/// name). For `Nix` the "packages" are nixpkgs attribute names — the caller renders them
/// into a `configuration.nix` snippet rather than installing (see execute.packages).
pub fn groups(mgr: PackageManager, release: Option<&str>) -> Vec<PackageGroup> {
    match mgr {
        PackageManager::Dnf => {
            compositor_installer_process_packages_enumerate_fedora::groups(release)
        }
        PackageManager::Apt => {
            compositor_installer_process_packages_enumerate_debian::groups(release)
        }
        PackageManager::Pacman => {
            compositor_installer_process_packages_enumerate_arch::groups(release)
        }
        PackageManager::Nix => {
            compositor_installer_process_packages_enumerate_nixos::groups(release)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_pkgs(mgr: PackageManager, release: Option<&str>) -> Vec<&'static str> {
        groups(mgr, release)
            .into_iter()
            .find(|g| g.key == "runtime")
            .expect("runtime group")
            .packages
    }

    #[test]
    fn every_manager_has_the_standard_group_keys() {
        for mgr in [PackageManager::Dnf, PackageManager::Apt, PackageManager::Pacman, PackageManager::Nix] {
            let keys: Vec<&str> = groups(mgr, None).iter().map(|g| g.key).collect();
            assert!(keys.contains(&"runtime"), "{mgr:?} missing runtime");
            assert!(keys.contains(&"xwayland"), "{mgr:?} missing xwayland");
            assert!(keys.contains(&"devtool"), "{mgr:?} missing devtool");
        }
    }

    #[test]
    fn apt_uses_runtime_lib_names_not_dev() {
        let pkgs = runtime_pkgs(PackageManager::Apt, Some("13"));
        for want in ["libgbm1", "libvulkan1", "libwayland-client0", "ffmpeg", "libpixman-1-0"] {
            assert!(pkgs.contains(&want), "apt runtime missing {want}: {pkgs:?}");
        }
        // No -dev headers leak into the runtime group.
        assert!(!pkgs.iter().any(|p| p.ends_with("-dev")), "apt runtime has -dev: {pkgs:?}");
    }

    #[test]
    fn apt_display_info_soversion_tracks_release() {
        // bookworm(-backports) + trixie ship libdisplay-info2; noble …1; resolute …3.
        assert!(runtime_pkgs(PackageManager::Apt, Some("12")).contains(&"libdisplay-info2"));
        assert!(runtime_pkgs(PackageManager::Apt, Some("13")).contains(&"libdisplay-info2"));
        assert!(runtime_pkgs(PackageManager::Apt, Some("24.04")).contains(&"libdisplay-info1"));
        assert!(runtime_pkgs(PackageManager::Apt, Some("26.04")).contains(&"libdisplay-info3"));
    }

    #[test]
    fn pacman_and_nix_have_their_loaders() {
        assert!(runtime_pkgs(PackageManager::Pacman, None).contains(&"vulkan-icd-loader"));
        assert!(runtime_pkgs(PackageManager::Pacman, None).contains(&"mesa"));
        assert!(runtime_pkgs(PackageManager::Nix, None).contains(&"vulkan-loader"));
    }
}
