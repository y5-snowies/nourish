//! What the compositor remembers for session management.
//!
//! The protocol makes the compositor the store: a client hands back an opaque
//! session id plus a per-toplevel name, and we are the side that knows what
//! that pair meant. y5 already holds the interesting half of that state on the
//! placeholder (geometry, launch plan), so this crate keeps the identity index
//! — which names exist in which session — plus the surface-data write that
//! carries the identity to the placeholder matcher.
//!
//! Two distinct lifetimes live here, and conflating them is the easy mistake:
//!
//! * **Remembered** names (`sessions`) outlive the client. That is the whole
//!   point — a name must still be known after the app exits so the next run's
//!   `restore_toplevel` can match it.
//! * **Live** grants (`granted`, `added`) last only as long as the protocol
//!   objects. They exist to answer "is this name already taken *right now*",
//!   which is what the `name_in_use` / `already_added` errors are about, and
//!   they must be released when those objects die.

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
        states.data_map.insert_if_missing_threadsafe(|| Mutex::new(identity.clone()));
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

/// Why a toplevel could not be filed under a session. Each maps to a protocol
/// error in both namespaces.
#[derive(Debug, PartialEq, Eq)]
pub enum GrantError {
    /// Another LIVE toplevel already holds this name in this session.
    NameInUse,
    /// This toplevel object is already in this session under some name.
    AlreadyAdded,
}

/// Every session id we have minted, and the toplevel names known in each.
///
/// Deliberately unbounded: the protocol leaves eviction to the compositor, and
/// picking a cap here would be inventing policy. Entries are removed only when
/// a client says so (`remove` / `remove_toplevel`) — never on a guess about
/// how many sessions are "too many".
#[derive(Default)]
pub struct SessionStore {
    /// Durable: session_id -> remembered toplevel names. Survives the client.
    sessions: HashMap<String, HashSet<String>>,
    /// Live: (session_id, name) -> the toplevel protocol id currently holding it.
    granted: HashMap<(String, String), u32>,
    /// Live: (session_id, toplevel protocol id) pairs already in the session.
    added: HashSet<(String, u32)>,
}

impl SessionStore {
    /// Whether we have ever minted or re-opened this session id.
    pub fn exists(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// Whether this session remembers a toplevel by this name.
    pub fn knows(&self, session_id: &str, name: &str) -> bool {
        self.sessions.get(session_id).is_some_and(|n| n.contains(name))
    }

    /// Open (or re-open) a session id so toplevels can be filed under it.
    pub fn open(&mut self, session_id: &str) {
        self.sessions.entry(session_id.to_string()).or_default();
    }

    /// File a toplevel name under a session and take the live grant.
    ///
    /// Returns `Err` when the client's bookkeeping conflicts with what is
    /// already live; the caller turns that into the matching protocol error.
    pub fn grant(&mut self, session_id: &str, name: &str, toplevel: u32) -> Result<(), GrantError> {
        let key = (session_id.to_string(), name.to_string());
        if let Some(held) = self.granted.get(&key) {
            // Re-granting the SAME toplevel the same name is a no-op, not a
            // conflict — a client may legitimately restore then re-add.
            if *held != toplevel {
                return Err(GrantError::NameInUse);
            }
        }
        if !self.added.insert((session_id.to_string(), toplevel)) {
            return Err(GrantError::AlreadyAdded);
        }
        self.granted.insert(key, toplevel);
        self.sessions.entry(session_id.to_string()).or_default().insert(name.to_string());
        Ok(())
    }

    /// `rename`: move a live grant to a new name, keeping the remembered state.
    pub fn rename(&mut self, session_id: &str, from: &str, to: &str) -> Result<(), GrantError> {
        let from_key = (session_id.to_string(), from.to_string());
        let to_key = (session_id.to_string(), to.to_string());
        let Some(toplevel) = self.granted.get(&from_key).copied() else { return Ok(()) };
        if self.granted.get(&to_key).is_some_and(|held| *held != toplevel) {
            return Err(GrantError::NameInUse);
        }
        self.granted.remove(&from_key);
        self.granted.insert(to_key, toplevel);
        if let Some(names) = self.sessions.get_mut(session_id) {
            names.remove(from);
            names.insert(to.to_string());
        }
        Ok(())
    }

    /// `remove_toplevel` / `xx_toplevel_session_v1.remove`: drop one name's
    /// stored state AND its live grant.
    pub fn forget(&mut self, session_id: &str, name: &str) {
        if let Some(names) = self.sessions.get_mut(session_id) {
            names.remove(name);
        }
        if let Some(toplevel) = self.granted.remove(&(session_id.to_string(), name.to_string())) {
            self.added.remove(&(session_id.to_string(), toplevel));
        }
    }

    /// A toplevel-session object died: release its live grant but KEEP the
    /// remembered name, which is exactly what makes the next run restorable.
    pub fn release(&mut self, session_id: &str, name: &str) {
        if let Some(toplevel) = self.granted.remove(&(session_id.to_string(), name.to_string())) {
            self.added.remove(&(session_id.to_string(), toplevel));
        }
    }

    /// `remove` (destructor): drop the session and everything under it.
    pub fn remove(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
        self.granted.retain(|(s, _), _| s != session_id);
        self.added.retain(|(s, _)| s != session_id);
    }
}
