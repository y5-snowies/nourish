//! Distro / package-manager detection from `/etc/os-release`.
//!
//! Every multiarch bundle ships the same `y5-install`, so the installer must learn at
//! runtime which package manager to drive. We classify the running distro by its
//! `os-release` `ID`/`ID_LIKE` into one of three manager families and expose the
//! `VERSION_ID` (used only to resolve the few version-suffixed apt package names).
//! Pure std.

/// The system package manager the installer will drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageManager {
    /// Fedora / RHEL family — `dnf` (+ optional RPM Fusion).
    Dnf,
    /// Debian / Ubuntu family — `apt-get`.
    Apt,
    /// Arch family — `pacman`.
    Pacman,
    /// NixOS — declarative, non-FHS. There is NO transactional install: the prebuilt
    /// binaries won't even find their dynamic loader without `nix-ld`, so the installer
    /// prints a `configuration.nix` snippet (nix-ld + runtime libs) to add, then how to
    /// apply it, instead of running a package command. See execute.packages.
    Nix,
}

impl PackageManager {
    /// Detect the package manager from `/etc/os-release` (`ID`, then `ID_LIKE`). Falls
    /// back to `Dnf` (the historical Fedora-only behavior) with a warning when the file
    /// is unreadable or the distro is unrecognized — the bundle is ABI-bound to its build
    /// distro anyway, so a mismatch here means the binary wouldn't run regardless.
    pub fn detect() -> PackageManager {
        let os = read_os_release();
        let id = field(&os, "ID").unwrap_or_default();
        let like = field(&os, "ID_LIKE").unwrap_or_default();
        match classify(&id).or_else(|| classify_like(&like)) {
            Some(m) => m,
            None => {
                eprintln!(
                    "warning: could not identify the distro (ID='{id}', ID_LIKE='{like}') — \
                     assuming dnf/Fedora. Set the right base or install packages manually."
                );
                PackageManager::Dnf
            }
        }
    }

    /// Parse a manager name (`dnf`/`apt`/`pacman`/`nix`, or a distro alias) — for the
    /// `--emit-packages` CLI flag and the pre-CI package-name verifier. `None` if unknown.
    pub fn parse(s: &str) -> Option<PackageManager> {
        match s.to_lowercase().as_str() {
            "dnf" | "fedora" | "rhel" => Some(PackageManager::Dnf),
            "apt" | "apt-get" | "debian" | "ubuntu" => Some(PackageManager::Apt),
            "pacman" | "arch" => Some(PackageManager::Pacman),
            "nix" | "nixos" => Some(PackageManager::Nix),
            _ => None,
        }
    }

    /// Human name of the underlying command, for messages. NixOS has no single install
    /// command (it's declarative) — reported as `nix` for display only.
    pub fn command(self) -> &'static str {
        match self {
            PackageManager::Dnf => "dnf",
            PackageManager::Apt => "apt-get",
            PackageManager::Pacman => "pacman",
            PackageManager::Nix => "nix",
        }
    }
}

/// The `VERSION_ID` from `/etc/os-release` (e.g. `"12"`, `"13"`, `"24.04"`), if present.
/// Used to pick the version-suffixed apt package names across Debian releases.
pub fn release_id() -> Option<String> {
    field(&read_os_release(), "VERSION_ID")
}

/// Map a single `ID` token to a manager. `None` if unrecognized.
fn classify(id: &str) -> Option<PackageManager> {
    match id {
        "fedora" | "rhel" | "centos" | "rocky" | "almalinux" => Some(PackageManager::Dnf),
        "debian" | "ubuntu" | "linuxmint" | "pop" => Some(PackageManager::Apt),
        "arch" | "archarm" | "manjaro" | "endeavouros" => Some(PackageManager::Pacman),
        "nixos" => Some(PackageManager::Nix),
        _ => None,
    }
}

/// `ID_LIKE` is a space-separated list of parent distros; take the first that classifies.
fn classify_like(like: &str) -> Option<PackageManager> {
    like.split_whitespace().find_map(classify)
}

/// Read `/etc/os-release`; empty string if unreadable (→ falls back to Dnf).
fn read_os_release() -> String {
    std::fs::read_to_string("/etc/os-release").unwrap_or_default()
}

/// Extract a `KEY=value` field from os-release, stripping surrounding quotes.
fn field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim().trim_matches('"').to_lowercase())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_distros() {
        assert_eq!(classify("fedora"), Some(PackageManager::Dnf));
        assert_eq!(classify("debian"), Some(PackageManager::Apt));
        assert_eq!(classify("ubuntu"), Some(PackageManager::Apt));
        assert_eq!(classify("arch"), Some(PackageManager::Pacman));
        assert_eq!(classify("nixos"), Some(PackageManager::Nix));
        assert_eq!(classify("void"), None);
    }

    #[test]
    fn falls_back_to_id_like() {
        // A derivative unknown by ID but with a known ID_LIKE parent.
        assert_eq!(classify_like("ubuntu debian"), Some(PackageManager::Apt));
        assert_eq!(classify_like("rhel fedora"), Some(PackageManager::Dnf));
    }

    #[test]
    fn field_strips_quotes_and_lowercases() {
        let os = "ID=debian\nVERSION_ID=\"12\"\nID_LIKE=\n";
        assert_eq!(field(os, "ID").as_deref(), Some("debian"));
        assert_eq!(field(os, "VERSION_ID").as_deref(), Some("12"));
        assert_eq!(field(os, "ID_LIKE"), None); // empty → None
    }
}
