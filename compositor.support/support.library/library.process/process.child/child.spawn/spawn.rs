//! `spawn()` / `status()` / `output()` — every spawn goes through one of them.
//!
//! [`resolve`] is what these add over `Command`'s own methods: a launch plan naming a
//! binary that isn't installed is the ordinary failure, and answering it BEFORE the fork
//! turns it into a returned error instead of an `exec` that fails in the child.
//!
//! There used to be a second job here, a mutex held across every spawn. It existed
//! because the launch reaper collected ANY child with `waitpid(-1)`, and `Command::spawn`
//! waits on its own child after a failed `exec` under an assertion — so a reaper that got
//! there first left that wait with `ECHILD` and PANICKED the spawning thread. The same
//! indiscriminate wait could take a child out from under [`status`] and [`output`] after
//! their spawn returned, which the lock never covered, and out from under smithay's
//! Xwayland reaper, which it could not cover.
//!
//! None of that exists now: a launched child is named by its own exit descriptor and
//! waited on by name (`child.pidfd`), so nothing in this process collects a child it does
//! not own, and there is nothing left to serialise.

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};

use compositor_support_library_process_child_pidfd::pidfd;

/// Spawn `cmd`, refusing a program `exec` could not have resolved.
///
/// PRIVATE, so that no caller can spawn without answering the question the two public
/// entry points differ on: who waits on this child. There is no neutral `spawn` on
/// purpose — the version that existed was the one every leak went through, because
/// forgetting to wait looks exactly like deciding not to.
fn spawn_inner(cmd: &mut Command) -> io::Result<Child> {
    if resolve(cmd).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("program not found or not executable: {}", cmd.get_program().to_string_lossy()),
        ));
    }
    cmd.spawn()
}

/// Spawn a child NOBODY will wait on, and arrange for it to be reaped anyway.
///
/// The variant for a caller that spawns and forgets: a one-shot `kill`, the input method,
/// a terminal. Those children still become zombies when they exit, and the difference
/// between them and the ones below is only who does the waiting — so the choice is made
/// HERE, at the call site, rather than by a reaper guessing from a distance. That guess
/// is what `waitpid(-1)` used to be, and what made it collide with every caller that did
/// wait on its own child.
///
/// The `Child` is returned so a caller can still take its stdio; dropping it changes
/// nothing, because the exit is collected through the descriptor rather than the handle.
pub fn spawn_detached(cmd: &mut Command) -> io::Result<Child> {
    let child = spawn_inner(cmd)?;
    pidfd::watch(child.id());
    Ok(child)
}

/// Spawn a child the CALLER will wait on, and hand back the handle.
///
/// The counterpart of [`spawn_detached`], and the promise in the name is the whole
/// contract: nothing else will collect this child, so a caller that returns without
/// waiting leaks a zombie for the life of the session. Take its stdio, drive it, and
/// `wait`, `try_wait` or `kill` it.
///
/// Use [`spawn_detached`] instead the moment the answer becomes "nobody waits".
pub fn spawn_awaited(cmd: &mut Command) -> io::Result<Child> {
    spawn_inner(cmd)
}

/// [`spawn_awaited`] then wait, replacing `Command::status` (which spawns unguarded).
pub fn status(cmd: &mut Command) -> io::Result<ExitStatus> {
    spawn_inner(cmd)?.wait()
}

/// [`spawn_awaited`] then collect, replacing `Command::output`. Stdio is FORCED, where
/// std's version only fills what the caller left unset — we cannot read a `Command`'s
/// stdio back to tell. Anything else wants [`spawn_awaited`] and a child driven by hand.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn_inner(cmd)?.wait_with_output()
}

/// Where `exec` would find this command's program: the path itself when it
/// contains a `/`, else the first executable of that name along `PATH` (the
/// command's own override when it sets one, otherwise ours). Relative paths are
/// resolved against the command's `current_dir`, exactly as the child will see
/// them. `None` means nothing would have been executed.
pub fn resolve(cmd: &Command) -> Option<PathBuf> {
    let program = cmd.get_program();
    let at = |candidate: PathBuf| {
        executable(match cmd.get_current_dir() {
            Some(dir) if candidate.is_relative() => dir.join(candidate),
            _ => candidate,
        })
    };
    if program.as_encoded_bytes().contains(&b'/') {
        return at(PathBuf::from(program));
    }
    let path = path_of(cmd).or_else(|| std::env::var_os("PATH"))?;
    std::env::split_paths(&path).find_map(|dir| at(dir.join(program)))
}

/// `Some(candidate)` when it is a file we could execute. Approximate by
/// necessity — a `noexec` mount or an LSM denial still fails at `exec`, which
/// is the guarded path and no longer a panic.
fn executable(candidate: PathBuf) -> Option<PathBuf> {
    let meta = std::fs::metadata(&candidate).ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(candidate)
}

/// The `PATH` this command would exec under, if it overrides ours.
fn path_of(cmd: &Command) -> Option<OsString> {
    cmd.get_envs().find_map(|(key, value)| (key == "PATH").then_some(value?.to_os_string()))
}
