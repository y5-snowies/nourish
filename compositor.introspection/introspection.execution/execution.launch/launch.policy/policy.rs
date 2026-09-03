//! The launch toggles. All compile-time `const`, so the branches not selected drop out
//! of the build entirely.

/// Which mechanism manages the lifecycle of a launched process.
///
/// REAPING IS NO LONGER ONE OF THE AXES. Every launch opens an exit descriptor for its
/// child and the loop collects that one child when it fires (`child.pidfd`), whichever
/// variant is selected, so none of these can leak a zombie any more. What is left to
/// choose is only whether the process is placed in a systemd scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchBackend {
    /// `Command::spawn` and nothing further: no scope, no cgroup of its own. Kept as the
    /// bisection baseline.
    Direct,
    /// Identical to [`Self::Direct`] today. It named the difference back when reaping was
    /// a SIGCHLD handler this variant opted into.
    DirectReaped,
    /// Self-spawn, then adopt the live PID into a transient systemd `.scope` over the
    /// session bus. A scope keeps us as the parent, so it changes the cgroup and nothing
    /// about who waits on the child.
    SystemdScope,
}

/// Where the spawn runs relative to the calloop thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchDispatch {
    /// Spawn synchronously on the calloop thread (historical).
    Inline,
    /// Spawn on a worker thread; the outcome is posted back into the calloop
    /// loop for the general `Executed` dispatch.
    OffThread,
}

/// Active process backend.
pub const LAUNCH_BACKEND: LaunchBackend = LaunchBackend::SystemdScope;

/// Active dispatch location.
pub const LAUNCH_DISPATCH: LaunchDispatch = LaunchDispatch::OffThread;

/// When `true`, a launch is expected to yield a PID and restoration records it
/// (the PID is a fallback behind the XDG activation token). When `false`, the
/// PID is discarded and restoration is token-only — used to exercise the token
/// correlation path in isolation.
pub const REQUIRE_PID: bool = true;
