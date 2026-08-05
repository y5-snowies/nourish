//! Bootstrap correlation: which session id do we mint for the client that a
//! placeholder just launched?
//!
//! `xdg-session-management-v1` has NO compositor→client channel for seeding a
//! session id. The client calls `get_session(reason, NULL)` and *we* choose the
//! string it will persist and hand back on every later run. So on that first
//! launch we have to work out which placeholder the connecting client belongs
//! to — and the only durable link at that moment is the pid we spawned.
//!
//! The placeholder layer [`register`]s the launch; [`resolve`] tries two
//! predicates, in the same order the restoration matchers use:
//!
//! 1. **Activation token.** The exact string we generated for this launch, read
//!    back out of the client's `/proc/<pid>/environ`. At `get_session` time the
//!    client may not have created a surface yet, so the protocol-level token is
//!    not reachable — environ is the only route, and it is the one
//!    `restoration.state/state.token` already leans on.
//! 2. **Pid tree.** The client's pid walked up its `/proc` parent chain.
//!
//! The token comes first because it survives what the pid chain does not: a
//! double-fork or `setsid` reparents the client to pid 1 and the walk dead-ends,
//! and a sandboxed app's pid tree does not line up with what we can see — but
//! the environment is inherited by the whole subtree either way. Neither helps
//! for a single-instance app whose window comes from a pre-existing process;
//! that limitation is the same one the matchers already carry.
//!
//! Crucially that id is the placeholder's EXISTING session id when it has one,
//! and only falls back to the placeholder uuid for a placeholder that has never
//! seen a session. Minting the uuid unconditionally would hand a returning
//! client a different string every generation, and its own per-session state
//! (the thing this protocol exists to let it restore) would never be found.
//!
//! Same class of signal the activation-token path already relies on, with the
//! same limits — but needed only while a placeholder has a launch in flight.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// How far up the process tree a client may sit from the pid we spawned.
/// Covers the usual shell / portal / wrapper hops without walking to pid 1.
const MAX_DEPTH: usize = 8;

/// How long a claim stays live. A client that has not asked for a session
/// within this window is not going to, and leaving the claim around would let
/// an unrelated later app inherit the placeholder's session id.
const TTL: Duration = Duration::from_secs(120);

struct Claim {
    pid: u32,
    /// The activation token passed to this launch, if any.
    token: String,
    placeholder: Uuid,
    /// The session id to mint for a client resolving to this claim.
    mint: String,
    at: Instant,
}

static CLAIMS: Mutex<Vec<Claim>> = Mutex::new(Vec::new());

/// Record a launch made on behalf of `placeholder`. `known_session` is the
/// session id that placeholder already restores under, if it has one; `token`
/// is the activation token the launch was given.
pub fn register(pid: u32, placeholder: Uuid, known_session: Option<String>, token: String) {
    let mint = known_session.unwrap_or_else(|| placeholder.to_string());
    let mut claims = CLAIMS.lock().unwrap_or_else(|e| e.into_inner());
    claims.retain(|c| c.at.elapsed() < TTL);
    claims.push(Claim { pid, token, placeholder, mint, at: Instant::now() });
}

/// Drop a placeholder's claims (it bound a window, or gave up).
pub fn release(placeholder: Uuid) {
    let mut claims = CLAIMS.lock().unwrap_or_else(|e| e.into_inner());
    claims.retain(|c| c.placeholder != placeholder && c.at.elapsed() < TTL);
}

/// The session id to mint for this wayland client, if it descends from a
/// placeholder launch. Token first, then the pid tree — see the module docs.
pub fn resolve(client_pid: i32) -> Option<String> {
    if client_pid <= 0 {
        return None;
    }
    let live: Vec<(u32, String, String)> = {
        let claims = CLAIMS.lock().unwrap_or_else(|e| e.into_inner());
        claims
            .iter()
            .filter(|c| c.at.elapsed() < TTL)
            .map(|c| (c.pid, c.token.clone(), c.mint.clone()))
            .collect()
    };
    if live.is_empty() {
        return None;
    }
    let pid0 = client_pid as u32;

    // 1. Activation token. An exact match on a string we generated for one
    //    launch, so a hit cannot be a coincidence.
    if let Some(token) = env_token(pid0) {
        if let Some((_, _, mint)) = live.iter().find(|(_, t, _)| !t.is_empty() && *t == token) {
            return Some(mint.clone());
        }
    }

    // 2. Pid tree.
    let mut pid = pid0;
    for _ in 0..MAX_DEPTH {
        if let Some((_, _, mint)) = live.iter().find(|(claimed, _, _)| *claimed == pid) {
            return Some(mint.clone());
        }
        pid = parent_of(pid)?;
        if pid <= 1 {
            break;
        }
    }
    None
}

/// The activation token in a process's environment. Both spellings are checked:
/// clients are known to consume one and leave the other behind.
fn env_token(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    raw.split(|b| *b == 0)
        .filter_map(|e| std::str::from_utf8(e).ok())
        .find_map(|e| {
            e.strip_prefix("XDG_ACTIVATION_TOKEN=")
                .or_else(|| e.strip_prefix("DESKTOP_STARTUP_ID="))
                .map(str::to_string)
        })
}

/// PPID from `/proc/<pid>/stat`. The `comm` field is parenthesised and may
/// itself contain spaces and parentheses, so split after its LAST `)`.
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = &stat[stat.rfind(')')? + 1..];
    tail.split_whitespace().nth(1)?.parse().ok()
}
