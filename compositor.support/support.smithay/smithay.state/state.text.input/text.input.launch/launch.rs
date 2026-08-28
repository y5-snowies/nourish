//! The compositor-owned input method: y5 launches exactly one IME process and grants the
//! input-method / virtual-keyboard globals ONLY to that process group, keyed by the spawned pid
//! PINNED BY ITS START-TIME (not `/proc/<pid>/exe`, which fails across namespaces, nor a spoofable
//! `comm`). The start-time is what makes pgid matching safe: after the IME dies its pid can be
//! recycled and a same-user process could take it + `setpgid(0,0)` to forge the group — but the
//! recycled pid has a different start-time, so it is rejected.
use std::io::BufRead;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use smithay::reexports::wayland_server::{Client, DisplayHandle};

use compositor_model_environment_preference_base::base::Ime;
use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;

/// pid (== process-group id) of the launched IME; `0` before launch / if none started.
static IME_PGID: AtomicI32 = AtomicI32::new(0);
/// `starttime` (`/proc/<pid>/stat` field 22) of that pid — pins the exact incarnation.
static IME_START: AtomicU64 = AtomicU64::new(0);

/// Launch the configured IME as a direct child in its OWN process group, record it as the sole
/// authorized IME, and stream its stderr to the log as errors. Call once at startup AFTER
/// `WAYLAND_DISPLAY` is exported (the child inherits it). Unset / empty `exec` launches nothing.
pub fn launch(configured: Option<Ime>) {
    let Some(ime) = configured.filter(|i| !i.exec.trim().is_empty()) else {
        info!("ime: none configured (preferences.json `ime`) — not launching");
        return;
    };

    // `process_group(0)` → new group with pgid == child pid (covers the IME + any helper it forks).
    // The IME must NOT daemonize (`-d`), or the connecting process is a reparented grandchild.
    let mut cmd = command(&ime.exec);
    cmd.args(&ime.args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let spawned = child_spawn::spawn(&mut cmd);
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return error!("ime: failed to launch '{}': {e}", ime.exec),
    };

    let pid = child.id() as i32;
    // Anchor start-time BEFORE publishing the pgid (`0` = unreadable → auth asserts liveness only).
    IME_START.store(stat_field(pid, 19).unwrap_or(0), Ordering::SeqCst);
    IME_PGID.store(pid, Ordering::SeqCst);
    info!("ime: launched '{}' (pid/pgid {pid})", ime.exec);

    // Detached (std `Child` never kills on drop; the global SIGCHLD reaper collects it); its stderr
    // surfaces on the console as errors.
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                error!("ime: {line}");
            }
        });
    }
}

/// Whether `client` is the launched IME — the ONLY client permitted to bind the input-method /
/// virtual-keyboard globals. Its process group (field 5) must equal the launched pid, and that pid
/// must still be our exact incarnation: alive AND matching start-time (field 22). A dead leader
/// (IME crashed) or a recycled pid revokes trust for everyone, closing the pid-reuse forgery.
pub fn is_authorized(client: &Client, dh: &DisplayHandle) -> bool {
    let auth = IME_PGID.load(Ordering::SeqCst);
    if auth == 0 {
        return false;
    }
    let now = stat_field(auth, 19);
    let want = IME_START.load(Ordering::SeqCst);
    if now.is_none() || (want != 0 && now != Some(want)) {
        return false;
    }
    let Ok(creds) = client.get_credentials(dh) else {
        return false;
    };
    creds.pid == auth || stat_field(creds.pid, 2) == Some(auth as u64)
}

/// A numeric `/proc/<pid>/stat` field, indexed from AFTER the last `)` so `comm` (which may contain
/// spaces / `)`) can't shift the count: index 0 = field 3. World-readable (no ptrace needed).
fn stat_field(pid: i32, index_after_comm: usize) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(index_after_comm)?.parse().ok()
}
