//! What the compositor remembers for `xdg_session_management_v1`.
//!
//! The protocol makes the compositor the store: a client hands back an opaque
//! session id plus a per-toplevel name, and we are the side that knows what
//! that pair meant. y5 already holds the interesting half of that state on the
//! placeholder (geometry, launch plan), so this crate keeps only the identity
//! index — which names exist in which session — plus the surface-data write
//! that carries the identity to the placeholder matcher.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;

/// The identity a client declared for one toplevel.
///
/// Written onto the toplevel's surface data at `add_toplevel` /
/// `restore_toplevel` — both of which the protocol requires BEFORE the first
/// commit — so by the time the window maps and the placeholder matcher runs,
/// the identity is already there to read. That is the whole point: the
/// activation token has to be recovered forensically after the fact, this
/// does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionIdentity {
    pub session_id: String,
    pub name: String,
}

/// Stamp (or overwrite, for `rename`) a toplevel's session identity.
pub fn set_identity(surface: &WlSurface, identity: SessionIdentity) {
    with_states(surface, |states| {
        states
            .data_map
            .insert_if_missing_threadsafe(|| Mutex::new(identity.clone()));
        if let Some(cell) = states.data_map.get::<Mutex<SessionIdentity>>() {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = identity;
        }
    });
}

/// Read a toplevel's session identity, if its client declared one.
pub fn identity(surface: &WlSurface) -> Option<SessionIdentity> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<Mutex<SessionIdentity>>()
            .map(|cell| cell.lock().unwrap_or_else(|e| e.into_inner()).clone())
    })
}

/// Every session id we have minted, and the toplevel names known in each.
///
/// Names decide whether `restore_toplevel` counts as a restore (name known →
/// emit `restored`) or is just `add_toplevel` under another spelling (unknown
/// name → no event). This index lives only as long as the compositor does;
/// the durable copy is the one the placeholder persists to disk.
#[derive(Default)]
pub struct SessionStore {
    sessions: HashMap<String, HashSet<String>>,
}

impl SessionStore {
    /// Whether we have ever minted or re-opened this session id.
    pub fn exists(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// Whether this session already knows a toplevel by this name.
    pub fn knows(&self, session_id: &str, name: &str) -> bool {
        self.sessions.get(session_id).is_some_and(|n| n.contains(name))
    }

    /// Open (or re-open) a session id so toplevels can be filed under it.
    pub fn open(&mut self, session_id: &str) {
        self.sessions.entry(session_id.to_string()).or_default();
    }

    /// File a toplevel name under a session.
    pub fn remember(&mut self, session_id: &str, name: &str) {
        self.sessions.entry(session_id.to_string()).or_default().insert(name.to_string());
    }

    /// `remove_toplevel`: drop one name's stored state.
    pub fn forget(&mut self, session_id: &str, name: &str) {
        if let Some(names) = self.sessions.get_mut(session_id) {
            names.remove(name);
        }
    }

    /// `xdg_session_v1.remove`: drop the session and everything under it.
    pub fn remove(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }
}
