//! `spawn()` / `status()` / `output()` — every spawn goes through one of them.
//!
//! Both hazards handled here follow from the SIGCHLD reaper
//! (`execution.launch/launch.reap`) collecting ANY child with `waitpid(-1)`:
//!
//! 1. **std reaps for us, under an assert.** When `exec` fails in the forked
//!    child, `Command::spawn` reads the failure off the CLOEXEC pipe and then
//!    waits on that child — `assert!(p.wait().is_ok(), "wait() should either
//!    return Ok or panic")`. A reaper that gets there first leaves that wait
//!    with `ECHILD`, and the assert PANICS on whatever thread was spawning.
//!    [`try_reap_guard`] and the lock [`spawn`] takes make the two exclusive.
//! 2. **The usual way to reach (1) need not fork at all.** A launch plan naming
//!    a binary that isn't installed is the ordinary case, so [`resolve`] answers
//!    it before the fork: a returned error instead of a race.

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Mutex, MutexGuard, TryLockError};

/// Held across every spawn below, and by the reaper for its whole drain, so a
/// spawn in flight cannot have its child collected out from under it. Held for
/// microseconds: a `fork` plus the exec handshake.
static SPAWNING: Mutex<()> = Mutex::new(());

/// The reaper's side of [`SPAWNING`]: `None` while a spawn holds it, so the
/// caller retries instead of blocking. Reaping has no latency requirement and
/// must never be what waits on a slow `exec` — a binary on a hung mount would
/// otherwise stall whichever thread reaps. A poisoned lock is taken anyway: the
/// data is `()`, and never reaping again is worse than anything a panicking
/// spawner could have left behind.
pub fn try_reap_guard() -> Option<MutexGuard<'static, ()>> {
    match SPAWNING.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// Spawn `cmd`, refusing a program `exec` could not have resolved.
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    if resolve(cmd).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("program not found or not executable: {}", cmd.get_program().to_string_lossy()),
        ));
    }
    let _spawning = SPAWNING.lock().unwrap_or_else(|e| e.into_inner());
    cmd.spawn()
}

/// [`spawn`] then wait, replacing `Command::status` (which spawns unguarded).
pub fn status(cmd: &mut Command) -> io::Result<ExitStatus> {
    spawn(cmd)?.wait()
}

/// [`spawn`] then collect, replacing `Command::output`. Stdio is FORCED, where
/// std's version only fills what the caller left unset — we cannot read a
/// `Command`'s stdio back to tell. Anything else wants [`spawn`] and a child
/// driven by hand.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn(cmd)?.wait_with_output()
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
