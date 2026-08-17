use std::path::{Path, PathBuf};

/// Where the prebuilt binaries and template files live in the unzipped artifact.
/// Resolved from `Y5_INSTALL_STAGE`, else the directory containing the installer
/// executable (the artifact root).
#[derive(Clone, Debug)]
pub struct Stage {
    pub root: PathBuf,
}

impl Stage {
    pub fn resolve() -> Stage {
        if let Ok(s) = std::env::var("Y5_INSTALL_STAGE") {
            return Stage { root: PathBuf::from(s) };
        }
        let root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Stage { root }
    }

    pub fn binary(&self, name: &str) -> PathBuf {
        self.root.join("binaries").join(name)
    }
    pub fn template(&self, rel: &str) -> PathBuf {
        self.root.join("templates").join(rel)
    }
}

/// Content for a file to be placed: copied from the stage, or written inline.
#[derive(Clone, Debug)]
pub enum Source {
    /// Copy a prebuilt file from the staging directory.
    Copy(PathBuf),
    /// Write generated text.
    Text(String),
}

/// One unit of installation work.
#[derive(Clone, Debug)]
pub enum Action {
    /// Place a file at `dest` (creating parent dirs). `root` = needs privilege.
    Place { dest: PathBuf, source: Source, mode: u32, root: bool },
    /// Run `systemctl --user <args>` for the invoking user.
    SystemctlUser(Vec<String>),
    /// Reload udev rules (root).
    UdevReload,
}

/// The home directory of the user the session will run as.
pub fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/root"))
}

/// The per-user systemd unit directory.
pub fn user_systemd_dir() -> PathBuf {
    home().join(".config/systemd/user")
}

/// `Action::Place` shorthand.
pub fn place(dest: PathBuf, source: Source, mode: u32, root: bool) -> Action {
    Action::Place { dest, source, mode, root }
}

/// True when the current process is uid 0. No libc dep — asks `id -u`.
pub fn is_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

/// Where settings.json is seeded (root=false → the invoking user's $HOME, the reason the
/// installer runs unprivileged).
pub fn settings_path() -> PathBuf {
    home().join(".config/y5.compositor/settings.json")
}

/// Whether the user already has a settings.json — callers SEED ONLY WHEN ABSENT.
///
/// Every other [`place`] in the plan is a binary, unit or desktop entry the installer
/// owns and must refresh. This one file belongs to the USER, and `place` is an
/// unconditional `install` + `mv -f`, so seeding it on a re-install would reset
/// per-machine tuning (`scanout_node`, `renderer_sync`, the capture settings) to
/// defaults. That was tolerable while re-installing was a deliberate act; it is not once
/// `y5.compositor.update` re-runs the installer unattended. A file predating a schema
/// bump needs no rewrite either: `config.base::migrate` lifts it in memory on load.
pub fn settings_exists() -> bool {
    settings_path().exists()
}
