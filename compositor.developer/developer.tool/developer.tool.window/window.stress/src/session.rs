//! `xdg_session_management_v1` client bindings + the subject's tiny session store.
//!
//! The `wayland-protocols` crate ships this staging XML but does not generate
//! bindings for it, so the XML is vendored under `protocols/` and scanned here.
//!
//! ## What the subject proves with it
//!
//! A placeholder relaunches the subject by replaying the argv it captured, so
//! anything recoverable from the command line proves nothing about the session
//! protocol. The store below is therefore deliberately keyed by the session id
//! the COMPOSITOR mints and holds a value that never appears in argv, in the
//! title, or anywhere the compositor could have re-derived it: if the value
//! comes back after a relaunch from a placeholder, it came back because
//! the compositor handed the process the same session id, and nothing else.

pub mod protocol {
    #![allow(non_camel_case_types, non_upper_case_globals, non_snake_case)]
    #![allow(dead_code, unused_imports, unused_variables, clippy::all)]

    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::xdg::shell::client::*;
    // `generate_interfaces!` emits `wayland_backend::protocol::…` paths.
    use wayland_backend;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::xdg::shell::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/xdg-session-management-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/xdg-session-management-v1.xml");
}

/// The same protocol under its pre-rename `xx_` namespace — the one GTK 4.22
/// binds, and the one no shipping app can currently reach (GTK's opt-in landed
/// after 4.22.4). Not wire-compatible with `xdg_`: `xx_toplevel_session_v1`
/// request 1 is `remove()` where `xdg_` has `rename(name)`, its `restored`
/// carries the `xdg_toplevel`, and `xx_session_v1` has no `remove_toplevel`.
///
/// Selected with `--xx`, so the subject can exercise whichever namespace you
/// want to test.
pub mod legacy {
    #![allow(non_camel_case_types, non_upper_case_globals, non_snake_case)]
    #![allow(dead_code, unused_imports, unused_variables, clippy::all)]

    use wayland_backend;
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::xdg::shell::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::xdg::shell::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/xx-session-management-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/xx-session-management-v1.xml");
}

use std::io::Write as _;
use std::path::{Path, PathBuf};

pub use protocol::xdg_session_manager_v1::{self, XdgSessionManagerV1};
pub use protocol::xdg_session_v1::{self, XdgSessionV1};
pub use protocol::xdg_toplevel_session_v1::{self, XdgToplevelSessionV1};

pub use legacy::xx_session_manager_v1::{self, XxSessionManagerV1};
pub use legacy::xx_session_v1::{self, XxSessionV1};
pub use legacy::xx_toplevel_session_v1::{self, XxToplevelSessionV1};

/// Which namespace the subject binds. They are separate globals, so a compositor
/// may advertise one, both or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    /// `xdg_session_manager_v1` — current wayland-protocols staging name.
    Xdg,
    /// `xx_session_manager_v1` — pre-rename name, what GTK 4.22 binds.
    Xx,
}

impl Namespace {
    pub fn label(self) -> &'static str {
        match self {
            Namespace::Xdg => "xdg",
            Namespace::Xx => "xx",
        }
    }
    pub fn global(self) -> &'static str {
        match self {
            Namespace::Xdg => "xdg_session_manager_v1",
            Namespace::Xx => "xx_session_manager_v1",
        }
    }
}

/// How the compositor answered our `get_session`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    /// No session global, or `--no-session`.
    Absent,
    /// Asked, still waiting for `created` / `restored`.
    Pending,
    /// The compositor did not know this session — a fresh value was generated.
    Created,
    /// The compositor knew the session id AND the toplevel name: our value came
    /// back off disk. This is the state the harness is looking for.
    Restored,
}

impl SessionPhase {
    pub fn label(self) -> &'static str {
        match self {
            SessionPhase::Absent => "ABSENT",
            SessionPhase::Pending => "PENDING",
            SessionPhase::Created => "NEW",
            SessionPhase::Restored => "RESTORED",
        }
    }
}

/// Everything the subject knows about its own session.
pub struct SessionState {
    pub phase: SessionPhase,
    /// The id the compositor minted or accepted.
    pub id: Option<String>,
    /// The value keyed by that id. NEVER passed on the command line.
    pub value: Option<String>,
    /// Our name for the toplevel inside the session.
    pub name: String,
    pub store: PathBuf,
    /// Which namespace we bound. Shown in the overlay so a run against a
    /// compositor advertising both is unambiguous.
    pub ns: Namespace,
}

impl SessionState {
    pub fn new(name: String, store: PathBuf, ns: Namespace, enabled: bool) -> Self {
        SessionState {
            phase: if enabled { SessionPhase::Pending } else { SessionPhase::Absent },
            id: None,
            value: None,
            name,
            store,
            ns,
        }
    }

    /// Resolve our value for `id`: whatever was stored under it last run, or a
    /// freshly minted one written back now. `restored` reports which happened —
    /// note it is the STORE's answer, independent of the compositor's `restored`
    /// event, so the two agreeing is itself part of the check.
    pub fn bind(&mut self, id: String) -> bool {
        let existing = load(&self.store, &id);
        let restored = existing.is_some();
        let value = existing.unwrap_or_else(|| {
            let v = mint_value();
            save(&self.store, &id, &v);
            v
        });
        self.id = Some(id);
        self.value = Some(value);
        restored
    }

    /// One-line summary for the overlay.
    pub fn overlay(&self) -> String {
        let id = self.id.as_deref().unwrap_or("-");
        let short: String = id.chars().take(8).collect();
        format!(
            "SESSION[{}] {} [{}] VALUE {}",
            self.ns.label(),
            short,
            self.phase.label(),
            self.value.as_deref().unwrap_or("-")
        )
    }
}

/// Default store path. Under `XDG_STATE_HOME` (or `/tmp`) so it survives the
/// process but is obviously throwaway.
pub fn default_store() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("y5-window-stress-sessions.txt")
}

/// `session_id<TAB>value` per line. Flat text on purpose — no deps, and you can
/// `cat` it to see exactly what the subject should be showing.
///
/// `save` appends, so a re-run leaves an older line for the same id in place;
/// taking the FIRST match keeps the value that was minted first, which is the
/// one the check expects to see come back. Malformed lines are skipped rather
/// than ending the search — a half-written line must not silently look like
/// "no stored value" and turn a genuine RESTORED into a NEW.
fn load(path: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(k, _)| *k == id)
        .map(|(_, v)| v.to_string())
}

fn save(path: &Path, id: &str, value: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut f = match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    // ONE `write_all` for the whole line, never `writeln!`.
    //
    // `write!`/`writeln!` issue a separate `write()` syscall per format
    // fragment, so `{id}`, `\t`, `{value}` and `\n` go out as four writes.
    // O_APPEND makes each of those atomic individually — not the line — so
    // several subjects spawning at once interleave into garbage like
    // `idAidBidC\t\t\tvalAvalBvalC`, and every id in that mess is then
    // unfindable. The subject mints a fresh value on the next run and the
    // restore check fails for reasons that have nothing to do with the
    // compositor. A single write of one buffer appends atomically.
    let _ = f.write_all(format!("{id}\t{value}\n").as_bytes());
}

/// A value with no relationship to argv, the title, or anything the compositor
/// can see — so recovering it can only be explained by the session id.
///
/// Drawn from `/dev/urandom` rather than time+pid: subjects spawned together
/// land within a millisecond of each other, and time-derived values came out
/// near-identical (`C21FC290`, `C20169CF`, `C203C2FB`) — which is useless when
/// the whole check is "is this the SAME value as before" read off three
/// overlays. Falls back to time+pid only if urandom is unreadable.
fn mint_value() -> String {
    // `read_exact` of 4 bytes — NOT `fs::read`, which reads to EOF and
    // /dev/urandom never reaches one.
    let mut buf = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if std::io::Read::read_exact(&mut f, &mut buf).is_ok() {
            return format!("{:08X}", u32::from_le_bytes(buf));
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    format!("{:08X}", (nanos ^ (std::process::id() as u64).rotate_left(17)) as u32)
}
