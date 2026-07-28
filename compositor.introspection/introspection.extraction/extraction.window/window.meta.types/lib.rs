//! Process/window metadata value types (split out of window.base `meta`).

/// Raw identity snapshots captured from Wayland surfaces and `/proc`.
pub mod types {
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Snapshot of a single process's identity, captured at one moment.
    /// All fields are optional because extraction can fail in pieces.
    #[derive(Debug, Default, Clone)]
    pub struct Meta {
        // Wayland surface metadata
        pub app_id: Option<String>,
        pub title: Option<String>,
        // Wayland client credentials
        pub pid: Option<u32>,
        pub uid: Option<u32>,
        pub gid: Option<u32>,
        // /proc-derived
        pub comm: Option<String>,
        pub exe: Option<PathBuf>,
        pub cmdline: Option<Vec<String>>,
        pub cwd: Option<PathBuf>,
        pub cgroup: Option<String>,
        pub selected_env: Option<HashMap<String, String>>,
    }

    /// A process tree node anchored on one Meta sample. `parent` walks upward
    /// (a chain, siblings not expanded); `children` is fully expanded down to
    /// a configured depth.
    #[derive(Debug, Clone)]
    pub struct MetaNode {
        pub meta: Meta,
        pub parent: Option<Box<MetaNode>>,
        pub children: Vec<MetaNode>,
    }

    impl MetaNode {
        pub fn leaf(meta: Meta) -> Self {
            Self { meta, parent: None, children: Vec::new() }
        }
    }
}

/// Environment variables captured from `/proc/<pid>/environ`: identity /
/// launcher attribution plus diagnostic session context.
pub mod env {
    pub const ENV_ALLOWLIST: &[&str] = &[
        // Launcher attribution
        "GIO_LAUNCHED_DESKTOP_FILE",
        "GIO_LAUNCHED_DESKTOP_FILE_PID",
        "XDG_ACTIVATION_TOKEN",
        "DESKTOP_STARTUP_ID",
        "BAMF_DESKTOP_FILE_HINT",
        // Sandbox identity
        "FLATPAK_ID",
        "SNAP",
        "SNAP_INSTANCE_NAME",
        "APPIMAGE",
        "container",
        // App-specific identity
        "CHROME_DESKTOP",
        // Game attribution — a Steam-launched process carries these whether it
        // reached us through xwayland or natively, and they survive into every
        // child, so the tree walk can find them on a wrapper script.
        "SteamAppId",
        "SteamGameId",
        "STEAM_COMPAT_DATA_PATH",
        "PROTON_ENABLE_WAYLAND",
        // Explicit user opt-in to tearing (settable as a Steam launch command).
        "Y5_TEARING",
        // Language runtimes
        "VIRTUAL_ENV",
        "CONDA_PREFIX",
        // Shell / terminal
        "TERM",
        "SHELL",
        "TERM_PROGRAM",
        "COLORTERM",
        // Session
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "DESKTOP_SESSION",
        // SSH (relevant for terminals)
        "SSH_CONNECTION",
        "SSH_CLIENT",
        "SSH_TTY",
    ];
}
