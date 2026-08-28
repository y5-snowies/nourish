//! Runtime lookups that `/proc` cannot answer: a container's NAME and whether
//! it is currently running. Both come from one `podman ps` call, cached.
//!
//! Threading is the whole design here. Extraction runs on the compositor's
//! calloop thread as well as the sampler thread, and spawning `podman` (tens to
//! hundreds of milliseconds) on the calloop thread would drop frames every time
//! a containerised window maps. So [`name_for`] NEVER blocks: it answers from
//! cache, and on a miss kicks off a detached refresh and returns `None`. The
//! sampler re-extracts a newly-registered placeholder on its very next tick, so
//! the name lands about a second later without anything having stalled.
//!
//! [`is_running_blocking`] is the opposite deal, for the launch path: a click is
//! a discrete user action, and the answer to "is this container up?" has to be
//! current, not up to 30s stale. It does NOT go through the cache — it asks
//! podman about that one container directly, because listing every container on
//! the machine to answer about one is work a click should not wait for. Both
//! blocking calls run under [`PROBE_TIMEOUT`], since both sit on the calloop
//! thread.

use std::collections::HashMap;
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;

/// How long a cached table is served before [`name_for`] triggers a refresh.
const TTL: Duration = Duration::from_secs(30);

/// How long the blocking probe waits for `podman ps` before giving up.
///
/// A healthy `podman ps` answers in milliseconds, so this is not a budget — it
/// is a fuse. It exists because the probe runs on the CALLOOP thread and
/// `Command::output()` has no timeout of its own: a wedged podman socket, a
/// contended c/storage lock, or a podman that is itself starting would freeze
/// the compositor for as long as it took, with no frames and no input.
///
/// Timing out reports "unknown", which the launch path reads as "no prompt
/// needed" and proceeds — the same answer it gives when podman is absent. That
/// degrades to a failed `podman exec` rather than a frozen desktop.
///
/// The better shape is asynchronous: probe off-thread and raise the container
/// prompt when the answer arrives, so the click never waits at all. That is a
/// larger change to the launch flow (the decision would stop being synchronous
/// with the press), so this bounds the damage in the meantime.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// How often the probe checks whether `podman ps` has finished.
const PROBE_POLL: Duration = Duration::from_millis(5);

/// One container as `podman ps` reported it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub running: bool,
}

#[derive(Default)]
struct Cache {
    entries: Vec<Entry>,
    refreshed_at: Option<Instant>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

/// Set while a detached refresh is in flight, so a burst of misses spawns one
/// `podman ps`, not one per window.
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Cached name for a container id. Never blocks; see the module note.
pub fn name_for(id: &str) -> Option<String> {
    let stale = {
        let cache = cache().lock().ok()?;
        let hit = cache.entries.iter().find(|e| e.id == id).map(|e| e.name.clone());
        if hit.is_some() {
            return hit;
        }
        cache.refreshed_at.map(|t| t.elapsed() > TTL).unwrap_or(true)
    };
    if stale {
        refresh_detached();
    }
    None
}

/// Current state of a container, looked up by id OR name. Blocks on `podman ps`.
///
/// `None` for an unknown or blank key — never a guess. The prefix arm exists
/// so a hand-typed short id resolves, and it is guarded on length: matching a
/// prefix of ANY length meant an empty key satisfied `starts_with` against the
/// first container in the list, so a blank field reported an unrelated
/// container's state and could prompt to start it.
pub fn is_running_blocking(key: &str) -> Option<bool> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    // Targeted, NOT the shared listing. This asks about exactly one container,
    // synchronously, on the calloop thread — and `podman ps --all` costs what the
    // whole machine holds, so on a host with a hundred containers it does a
    // hundred times the necessary work inside a click. The cache exists to
    // amortise NAME resolution across every window at once; that is a different
    // question and keeps the listing.
    //
    // Prefix matching comes free: podman resolves a short id itself, and better
    // than we did — it rejects an ambiguous prefix instead of taking the first
    // match, which is what the short-id guard here used to work around.
    let out = probe(&["container", "inspect", key, "--format", "{{.State.Running}}"])?;
    match out.trim() {
        "true" => Some(true),
        "false" => Some(false),
        // Unknown container, or a podman that answered something else. Never a
        // guess: the launch path reads `None` as "do not prompt".
        _ => None,
    }
}

/// Run `podman ps` now and replace the cache. Blocking.
pub fn refresh_blocking() -> bool {
    let Some(entries) = list() else { return false };
    let Ok(mut cache) = cache().lock() else { return false };
    cache.entries = entries;
    cache.refreshed_at = Some(Instant::now());
    true
}

fn refresh_detached() {
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return; // one in flight already
    }
    let spawned = std::thread::Builder::new()
        .name("y5-podman-ps".into())
        .spawn(|| {
            refresh_blocking();
            REFRESHING.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        REFRESHING.store(false, Ordering::Release);
    }
}

/// `podman ps` as a tab-separated table. `--no-trunc` is required: without it
/// `{{.ID}}` is the 12-char short id and would never equal the 64-char id the
/// cgroup gave us.
fn list() -> Option<Vec<Entry>> {
    let out = probe(&["ps", "--all", "--no-trunc", "--format", "{{.ID}}\t{{.Names}}\t{{.State}}"])?;
    Some(out.lines().filter_map(parse_row).collect())
}

/// Run `podman` with `args` under [`PROBE_TIMEOUT`], returning its stdout.
///
/// `None` on a non-zero exit, a spawn failure, or the timeout — all of which the
/// callers read as "unknown" rather than as an answer.
fn probe(args: &[&str]) -> Option<String> {
    let mut cmd = command("podman");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = child_spawn::spawn(&mut cmd).ok()?;
    // Spawned and polled rather than `output()`, which waits without a bound —
    // see `PROBE_TIMEOUT` for why an unbounded wait here freezes the compositor.
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() >= deadline => {
                // Killed rather than left running: a probe we have stopped
                // waiting for is a process nobody will ever read.
                let _ = child.kill();
                let _ = child.wait();
                warn!("container probe: podman {args:?} exceeded {PROBE_TIMEOUT:?} — reporting unknown");
                return None;
            }
            Ok(None) => std::thread::sleep(PROBE_POLL),
            Err(_) => return None,
        }
    }
    // Read after exit. These outputs are small, and one large enough to fill the
    // pipe would block the child instead — which the loop above sees as a timeout
    // and kills, so it degrades to "unknown" rather than deadlocking.
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

fn parse_row(line: &str) -> Option<Entry> {
    let mut fields = line.split('\t');
    let id = fields.next()?.trim();
    let name = fields.next()?.trim();
    let state = fields.next()?.trim();
    if id.is_empty() {
        return None;
    }
    // `{{.Names}}` is a []string: depending on the podman version it renders as
    // `name` or `[name name2]`. Take the first name either way.
    let name = name.trim_start_matches('[').trim_end_matches(']');
    let name = name.split_whitespace().next().unwrap_or("");
    Some(Entry {
        id: id.to_string(),
        name: name.to_string(),
        running: state.eq_ignore_ascii_case("running"),
    })
}
