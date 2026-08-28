//! Request / outcome records.

use uuid::Uuid;

/// A fully-resolved launch, ready to spawn. Built on the calloop thread and
/// safe to `Send` to the launch worker.
#[derive(Clone, Debug)]
pub struct LaunchRequest {
    /// Program path followed by its arguments (argv[0] is the program).
    pub argv: Vec<String>,
    /// Environment overlay applied on top of the inherited environment.
    pub env: Vec<(String, String)>,
    /// Working directory, if the plan specified one.
    pub working_dir: Option<String>,
    /// XDG activation token — the primary restoration correlation key.
    pub token: String,
    /// Unit/scope name used when the backend is `SystemdScope`.
    pub unit: String,
    /// Ties the outcome back to an originator (e.g. a placeholder uuid).
    /// `None` for launches nobody needs to correlate (plain placeholders).
    pub correlation: Option<Uuid>,
    /// Container (name, else id) to run this launch inside; `None` = host.
    ///
    /// Carried as DATA rather than applied at build time on purpose: the
    /// rewrite into `podman exec` turns `env` into `--env` flags, and `env` is
    /// not final until the Executor has prepended its `base_env`. Wrapping
    /// earlier silently dropped the live `WAYLAND_DISPLAY` (and the rest of
    /// base_env) from every containerised launch, applying it to the podman
    /// client instead — where it does nothing for the app inside.
    pub container: Option<String>,
    /// Start `container` before exec'ing into it. Only ever set from a
    /// confirmed answer to the "container is not running" prompt.
    pub start_container: bool,
}

/// The result of attempting a launch. Doubles as the payload of the general
/// `Executed` bus event.
#[derive(Clone, Debug)]
pub struct LaunchOutcome {
    pub correlation: Option<Uuid>,
    pub token: String,
    /// `Some` on success — we always self-spawn, so the PID is `Child::id()`.
    /// `None` only when the spawn itself failed.
    pub pid: Option<u32>,
    /// `Err` carries a human-readable reason.
    pub result: Result<(), String>,
}
