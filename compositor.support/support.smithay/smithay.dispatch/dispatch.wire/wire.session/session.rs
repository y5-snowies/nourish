//! Session management — durable per-toplevel identity.
//!
//! Hand-rolled like `color_impls` / `tablet_impls`: smithay has no module for
//! this protocol, and the `wayland-protocols` crate ships the staging XML
//! without generating bindings for it, so the XML is vendored next to this file
//! and scanned in [`protocol`]. The `Dispatch`/`GlobalDispatch` impls live in
//! `dispatch.state/state.base`, where `delegate_dispatch2!(Dispatch)` makes the
//! bounds provable.
//!
//! BOTH namespaces are implemented and advertised: `xdg_session_management_v1`
//! (current staging) in [`protocol`], and the pre-rename `xx_` spelling in
//! [`legacy`], which is what GTK 4.22 binds and therefore the only one a
//! shipping app speaks today. They are not wire-compatible — see [`legacy`] —
//! but they share one [`SessionStore`] and one claim registry, so which name a
//! client happened to bind is invisible to the placeholder path.
//!
//! ## What it buys y5
//!
//! The placeholder restoration path identifies a returning window forensically,
//! AFTER it maps — activation token off surface data or `/proc/<pid>/environ`,
//! then the pid tree, then hint equality. Every one of those answers "which
//! launch did this window come from?", a question that only has an answer for a
//! few seconds after we spawned something. This protocol answers "which window
//! is this?", which has an answer forever: the client hands back a session id
//! we minted plus its own name for the toplevel, before the first commit.
//!
//! ## Who mints the id
//!
//! The client. It calls `get_session(reason, NULL)` on first run and persists
//! whatever string we send in `created`. We choose that string — so when the
//! request comes from a client a placeholder launched (see
//! `state.session/session.claim`), we mint the placeholder's uuid and the pair
//! is bound from then on with no matching at all.
//!
//! ## Deliberate leniency
//!
//! The protocol specifies `name_in_use` / `already_added` / `already_mapped`
//! protocol errors for client bookkeeping mistakes. Raising one kills the
//! client, and every case is one where simply applying the newer identity is
//! harmless to us — so this implementation does not raise them. `invalid_reason`
//! IS raised: an unparseable enum means we cannot tell what the client wants.

pub mod protocol {
    //! Server bindings generated from the vendored staging XML.
    #![allow(non_camel_case_types, non_upper_case_globals, non_snake_case)]
    #![allow(dead_code, unused_imports, unused_variables, clippy::all)]

    use smithay::reexports::wayland_protocols::xdg::shell::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;
    // `generate_interfaces!` emits `wayland_backend::protocol::…` paths, so the
    // crate has to be nameable here as well as `wayland_server`.
    use wayland_backend;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols::xdg::shell::server::__interfaces::*;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("xdg-session-management-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("xdg-session-management-v1.xml");
}

/// The SAME protocol under its pre-rename `xx_` namespace — what GTK 4.22
/// actually binds, and therefore the only version any shipping app speaks today.
///
/// It is not a namespace alias, so it cannot share the `xdg_` dispatch:
///
/// * `xx_toplevel_session_v1` request 1 is `remove()` (destructor, no args);
///   the same opcode in `xdg_` is `rename(name: string)`. Aliasing would make
///   GTK's `remove` parse as a `rename` missing its argument.
/// * `xx_..._session_v1.restored` carries the `xdg_toplevel`; the `xdg_` one
///   carries nothing.
/// * `xx_session_v1` has no `remove_toplevel`, and both error enums renumber.
///
/// Only the request plumbing differs — both namespaces resolve against the same
/// [`SessionStore`] and mint through the same claim registry, so a client on
/// either one lands in the same placeholder identity.
pub mod legacy {
    //! Server bindings generated from the vendored `xx_` staging XML.
    #![allow(non_camel_case_types, non_upper_case_globals, non_snake_case)]
    #![allow(dead_code, unused_imports, unused_variables, clippy::all)]

    use smithay::reexports::wayland_protocols::xdg::shell::server::*;
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;
    use wayland_backend;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols::xdg::shell::server::__interfaces::*;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("xx-session-management-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("xx-session-management-v1.xml");
}

use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, Resource,
};
use smithay::wayland::shell::xdg::XdgShellSurfaceUserData;
use uuid::Uuid;

use compositor_support_smithay_state_session_store::store::{SessionIdentity, SessionStore};

pub use protocol::xdg_session_manager_v1::{self, XdgSessionManagerV1};
pub use protocol::xdg_session_v1::{self, XdgSessionV1};
pub use protocol::xdg_toplevel_session_v1::{self, XdgToplevelSessionV1};

pub use legacy::xx_session_manager_v1::{self, XxSessionManagerV1};
pub use legacy::xx_session_v1::{self, XxSessionV1};
pub use legacy::xx_toplevel_session_v1::{self, XxToplevelSessionV1};

use std::collections::HashMap;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Weak;

use compositor_support_smithay_state_session_store::store::GrantError;

/// Protocol version we advertise. Staging is at v1 in both namespaces.
pub const VERSION: u32 = 1;

/// User data of a bound session object: the id it resolved to.
#[derive(Debug)]
pub struct SessionData {
    pub session_id: String,
}

/// User data of a per-toplevel session handle, in either namespace.
///
/// `surface`/`toplevel` are WEAK. They are client objects held in
/// compositor-side state that outlives them, and a strong reference here would
/// mean this map decides when a client's objects die. Every use upgrades and
/// tolerates the miss.
#[derive(Debug)]
pub struct ToplevelSessionData {
    pub session_id: String,
    /// Mutable because `xdg_`'s `rename` re-keys the toplevel while keeping its
    /// state; the `xx_` namespace has no rename.
    pub name: std::sync::Mutex<String>,
    pub surface: Option<Weak<WlSurface>>,
    pub toplevel: Option<Weak<XdgToplevel>>,
    /// The owning `xdg_session_v1`, weakly. `rename` can fail with
    /// `name_in_use`, and that error is defined on the SESSION interface —
    /// `xdg_toplevel_session_v1` has no error enum at all — so the handle needs
    /// a way back to the object the error must be posted on. `None` on the
    /// `xx_` path, which has no rename.
    pub owner: Option<Weak<XdgSessionV1>>,
}

impl ToplevelSessionData {
    fn surface(&self) -> Option<WlSurface> {
        self.surface.as_ref().and_then(|w| w.upgrade().ok())
    }
}

// ── live session ownership ───────────────────────────────────────────────────
// Kept here rather than in the (protocol-free) store because `replaced` has to
// be SENT on a resource. This is the half of the protocol that was missing
// entirely: the spec says a session id has at most one live manager, that the
// same client asking twice is an `in_use` error, and that a different client
// taking over displaces the incumbent with `replaced`.

/// A live session manager object, in whichever namespace bound it.
#[derive(Debug, Clone)]
pub enum SessionResource {
    Xdg(XdgSessionV1),
    Xx(XxSessionV1),
}

impl SessionResource {
    fn client_id(&self) -> Option<ClientId> {
        match self {
            SessionResource::Xdg(r) => r.client().map(|c| c.id()),
            SessionResource::Xx(r) => r.client().map(|c| c.id()),
        }
    }
    fn send_replaced(&self) {
        match self {
            SessionResource::Xdg(r) => r.replaced(),
            SessionResource::Xx(r) => r.replaced(),
        }
    }
    fn same_as(&self, other: &SessionResource) -> bool {
        match (self, other) {
            (SessionResource::Xdg(a), SessionResource::Xdg(b)) => a.id() == b.id(),
            (SessionResource::Xx(a), SessionResource::Xx(b)) => a.id() == b.id(),
            _ => false,
        }
    }
}

/// session id -> the object currently managing it.
#[derive(Default)]
pub struct SessionLive {
    held: HashMap<String, SessionResource>,
}

impl SessionLive {
    /// Whether `client` is already the live manager of `session_id` — the
    /// `in_use` condition. Checked BEFORE creating the new object so we never
    /// build a resource only to immediately kill its client.
    pub fn held_by(&self, session_id: &str, client: &Client) -> bool {
        self.held.get(session_id).and_then(|r| r.client_id()) == Some(client.id())
    }

    /// Install `claimant` as the manager, displacing (and notifying) any
    /// incumbent from a different client.
    pub fn install(&mut self, session_id: &str, claimant: SessionResource) {
        if let Some(previous) = self.held.get(session_id) {
            previous.send_replaced();
            info!("session: {session_id} taken over; previous holder sent replaced");
        }
        self.held.insert(session_id.to_string(), claimant);
    }

    /// Release on destruction — but ONLY if `holder` is still the registered
    /// manager. A displaced object's destructor must not evict the client that
    /// displaced it.
    pub fn release(&mut self, session_id: &str, holder: &SessionResource) {
        if self.held.get(session_id).is_some_and(|cur| cur.same_as(holder)) {
            self.held.remove(session_id);
        }
    }
}

// ── xdg_ namespace ───────────────────────────────────────────────────────────

pub fn create_global<D>(dh: &DisplayHandle)
where
    D: GlobalDispatch<XdgSessionManagerV1, ()> + 'static,
{
    dh.create_global::<D, XdgSessionManagerV1, ()>(VERSION, ());
    info!("session: xdg_session_manager_v1 global advertised");
}

/// `get_session`: resolve or mint a session id, then tell the client which of
/// the two happened. `created` carries the string it must persist; `restored`
/// says the id it supplied was one we know.
pub fn dispatch_manager<D>(
    store: &mut SessionStore,
    live: &mut SessionLive,
    manager: &XdgSessionManagerV1,
    client: &Client,
    dh: &DisplayHandle,
    request: xdg_session_manager_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XdgSessionV1, SessionData> + 'static,
{
    let xdg_session_manager_v1::Request::GetSession { id, reason, session_id } = request else {
        return;
    };
    // Posted on the manager, not the new object: the session is still an
    // uninitialised `New<_>` and has no resource to error on.
    let Ok(reason) = reason.into_result() else {
        manager.post_error(
            xdg_session_manager_v1::Error::InvalidReason,
            "unknown xdg_session_manager_v1 reason",
        );
        return;
    };

    let resolved = session_id.filter(|s| store.exists(s));
    if let Some(existing) = resolved.as_deref() {
        if live.held_by(existing, client) {
            manager.post_error(
                xdg_session_manager_v1::Error::InUse,
                "session already in use by this client",
            );
            return;
        }
    }

    match resolved {
        // Known id — the client is coming back. Its toplevels can now restore
        // by name, which is the entire point of the protocol.
        Some(existing) => {
            let session = di.init(id, SessionData { session_id: existing.clone() });
            live.install(&existing, SessionResource::Xdg(session.clone()));
            session.restored();
            info!("session: restored session {existing} (reason {reason:?})");
        }
        // New (or unrecognised — the protocol says treat that as NULL) session.
        // When we can work out which placeholder launched this client, mint the
        // id THAT placeholder already restores under, so a client that asks
        // with NULL on every run still gets a stable string back.
        None => {
            let pid = client.get_credentials(dh).map(|c| c.pid).unwrap_or(-1);
            let minted = compositor_support_smithay_state_session_claim::claim::resolve(pid)
                .unwrap_or_else(|| Uuid::now_v7().to_string());
            store.open(&minted);
            let session = di.init(id, SessionData { session_id: minted.clone() });
            live.install(&minted, SessionResource::Xdg(session.clone()));
            session.created(minted.clone());
            info!("session: minted session {minted} for pid {pid} (reason {reason:?})");
        }
    }
}

/// `add_toplevel` / `restore_toplevel` / `remove_toplevel`.
///
/// Both add and restore stamp the identity onto the toplevel's surface data —
/// that write is what the placeholder matcher reads. They differ only in the
/// `restored` event, which restore emits when the name was already known.
pub fn dispatch_session<D>(
    store: &mut SessionStore,
    session: &XdgSessionV1,
    session_id: &str,
    request: xdg_session_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XdgToplevelSessionV1, ToplevelSessionData> + 'static,
{
    match request {
        xdg_session_v1::Request::AddToplevel { id, toplevel, name } => {
            if reject_name(session, &name) {
                return;
            }
            if let Err(e) = store.grant(session_id, &name, toplevel.id().protocol_id()) {
                post_grant_error(session, e, &name);
                return;
            }
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            di.init(id, data(session_id, &name, surface, Some(toplevel), Some(session)));
        }
        xdg_session_v1::Request::RestoreToplevel { id, toplevel, name } => {
            if reject_name(session, &name) {
                return;
            }
            let known = store.knows(session_id, &name);
            if let Err(e) = store.grant(session_id, &name, toplevel.id().protocol_id()) {
                post_grant_error(session, e, &name);
                return;
            }
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            let handle = di.init(id, data(session_id, &name, surface, Some(toplevel), Some(session)));
            if known {
                handle.restored();
                info!("session: restoring toplevel '{name}' of session {session_id}");
            }
        }
        xdg_session_v1::Request::RemoveToplevel { name } => store.forget(session_id, &name),
        // `remove` (destructor) drops the session's stored state entirely;
        // `destroy` keeps it, so only the former reaches the store.
        xdg_session_v1::Request::Remove => store.remove(session_id),
        _ => {}
    }
}

/// `rename`: re-key a live toplevel session, preserving its state.
pub fn dispatch_toplevel_session(
    store: &mut SessionStore,
    data: &ToplevelSessionData,
    request: xdg_toplevel_session_v1::Request,
) {
    let xdg_toplevel_session_v1::Request::Rename { name } = request else {
        return;
    };
    let mut current = data.name.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(GrantError::NameInUse) = store.rename(&data.session_id, &current, &name) {
        // Posted on the session, not this handle: `name_in_use` lives on the
        // session interface. If the session is already gone the rename simply
        // does not happen — there is nobody left to tell.
        if let Some(session) = data.owner.as_ref().and_then(|w| w.upgrade().ok()) {
            session.post_error(
                xdg_session_v1::Error::NameInUse,
                format!("toplevel name '{name}' already in use in this session"),
            );
        }
        return;
    }
    stamp(data.surface().as_ref(), &data.session_id, &name);
    *current = name;
}

// ── xx_ namespace (GTK 4.22, Qt 6.11, Chrome 151) ────────────────────────────
// Same store and claim registry, different wire shape — see `mod legacy`.

pub fn create_legacy_global<D>(dh: &DisplayHandle)
where
    D: GlobalDispatch<XxSessionManagerV1, ()> + 'static,
{
    dh.create_global::<D, XxSessionManagerV1, ()>(VERSION, ());
    info!("session: xx_session_manager_v1 global advertised (pre-rename namespace)");
}

/// `xx_session_manager_v1.get_session`. Mirrors [`dispatch_manager`], except the
/// `xx_` error enum has no `invalid_reason`, so an unparseable reason is logged
/// and treated as `launch` rather than killing the client.
pub fn dispatch_legacy_manager<D>(
    store: &mut SessionStore,
    live: &mut SessionLive,
    manager: &XxSessionManagerV1,
    client: &Client,
    dh: &DisplayHandle,
    request: xx_session_manager_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XxSessionV1, SessionData> + 'static,
{
    let xx_session_manager_v1::Request::GetSession { id, reason, session } = request else {
        return;
    };
    if reason.into_result().is_err() {
        warn!("session(xx): unknown reason, treating as launch");
    }

    let resolved = session.filter(|s| store.exists(s));
    if let Some(existing) = resolved.as_deref() {
        if live.held_by(existing, client) {
            manager
                .post_error(xx_session_manager_v1::Error::InUse, "session already in use by this client");
            return;
        }
    }

    match resolved {
        Some(existing) => {
            let s = di.init(id, SessionData { session_id: existing.clone() });
            live.install(&existing, SessionResource::Xx(s.clone()));
            s.restored();
            info!("session(xx): restored session {existing}");
        }
        None => {
            let pid = client.get_credentials(dh).map(|c| c.pid).unwrap_or(-1);
            let minted = compositor_support_smithay_state_session_claim::claim::resolve(pid)
                .unwrap_or_else(|| Uuid::now_v7().to_string());
            store.open(&minted);
            let s = di.init(id, SessionData { session_id: minted.clone() });
            live.install(&minted, SessionResource::Xx(s.clone()));
            s.created(minted.clone());
            info!("session(xx): minted session {minted} for pid {pid}");
        }
    }
}

/// `xx_session_v1.add_toplevel` / `restore_toplevel`. No `remove_toplevel` in
/// this namespace, and `restored` is emitted on the TOPLEVEL handle carrying
/// the `xdg_toplevel` back.
pub fn dispatch_legacy_session<D>(
    store: &mut SessionStore,
    session: &XxSessionV1,
    session_id: &str,
    request: xx_session_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XxToplevelSessionV1, ToplevelSessionData> + 'static,
{
    match request {
        xx_session_v1::Request::AddToplevel { id, toplevel, name } => {
            if let Err(e) = store.grant(session_id, &name, toplevel.id().protocol_id()) {
                post_legacy_grant_error(session, e, &name);
                return;
            }
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            di.init(id, data(session_id, &name, surface, Some(toplevel), None));
        }
        xx_session_v1::Request::RestoreToplevel { id, toplevel, name } => {
            let known = store.knows(session_id, &name);
            if let Err(e) = store.grant(session_id, &name, toplevel.id().protocol_id()) {
                post_legacy_grant_error(session, e, &name);
                return;
            }
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            let handle = di.init(id, data(session_id, &name, surface, Some(toplevel.clone()), None));
            if known {
                handle.restored(&toplevel);
                info!("session(xx): restoring toplevel '{name}' of session {session_id}");
            }
        }
        // `remove` (destructor) drops the session's stored state; `destroy` keeps it.
        xx_session_v1::Request::Remove => store.remove(session_id),
        _ => {}
    }
}

/// `xx_toplevel_session_v1.remove` — the destructor that ALSO forgets this
/// toplevel's stored state. This is the opcode that `xdg_` reuses for `rename`,
/// which is why the two namespaces cannot share a dispatch.
pub fn dispatch_legacy_toplevel_session(
    store: &mut SessionStore,
    data: &ToplevelSessionData,
    request: xx_toplevel_session_v1::Request,
) {
    if let xx_toplevel_session_v1::Request::Remove = request {
        let name = data.name.lock().unwrap_or_else(|e| e.into_inner()).clone();
        store.forget(&data.session_id, &name);
    }
}

// ── destruction ──────────────────────────────────────────────────────────────
// Every other hand-rolled protocol in `state.base` cleans up in `destroyed`;
// these previously did not, so live grants and session ownership leaked for the
// compositor's lifetime and a client could never re-take a name it had dropped.
//
// The asymmetry that matters: destroying a session or toplevel-session object
// releases the LIVE claim but must NOT forget the remembered name. Remembering
// past the client's death is precisely what makes the next run restorable —
// only an explicit `remove` / `remove_toplevel` erases it.

/// A session object died (client disconnect, `destroy`, or `remove`).
pub fn destroyed_session(live: &mut SessionLive, data: &SessionData, holder: SessionResource) {
    live.release(&data.session_id, &holder);
}

/// A toplevel-session object died: drop its live grant, keep its stored name.
pub fn destroyed_toplevel_session(store: &mut SessionStore, data: &ToplevelSessionData) {
    let name = data.name.lock().unwrap_or_else(|e| e.into_inner()).clone();
    store.release(&data.session_id, &name);
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// `xdg_` defines `invalid_name`; `xx_` does not, so the caller differs.
fn reject_name(session: &XdgSessionV1, name: &str) -> bool {
    if name.is_empty() {
        session.post_error(xdg_session_v1::Error::InvalidName, "empty toplevel name");
        return true;
    }
    false
}

fn post_grant_error(session: &XdgSessionV1, e: GrantError, name: &str) {
    match e {
        GrantError::NameInUse => session.post_error(
            xdg_session_v1::Error::NameInUse,
            format!("toplevel name '{name}' already in use in this session"),
        ),
        GrantError::AlreadyAdded => session
            .post_error(xdg_session_v1::Error::AlreadyAdded, "toplevel already added to this session"),
    }
}

/// The `xx_` enum has `name_in_use` but no `already_added`, so a double-add is
/// logged rather than fatal — there is no code to report it with.
fn post_legacy_grant_error(session: &XxSessionV1, e: GrantError, name: &str) {
    match e {
        GrantError::NameInUse => session.post_error(
            xx_session_v1::Error::NameInUse,
            format!("toplevel name '{name}' already in use in this session"),
        ),
        GrantError::AlreadyAdded => {
            warn!("session(xx): toplevel added twice under '{name}' — no error code in this namespace")
        }
    }
}

fn data(
    session_id: &str,
    name: &str,
    surface: Option<WlSurface>,
    toplevel: Option<XdgToplevel>,
    owner: Option<&XdgSessionV1>,
) -> ToplevelSessionData {
    ToplevelSessionData {
        session_id: session_id.to_string(),
        name: std::sync::Mutex::new(name.to_string()),
        surface: surface.map(|s| s.downgrade()),
        toplevel: toplevel.map(|t| t.downgrade()),
        owner: owner.map(|s| s.downgrade()),
    }
}

fn stamp(surface: Option<&WlSurface>, session_id: &str, name: &str) {
    let Some(surface) = surface else {
        warn!("session: toplevel resource with no shell user data — identity dropped");
        return;
    };
    compositor_support_smithay_state_session_store::store::set_identity(
        surface,
        SessionIdentity { session_id: session_id.to_string(), name: name.to_string() },
    );
}

/// The `wl_surface` behind a raw `xdg_toplevel` resource. Smithay hangs
/// `XdgShellSurfaceUserData` off every toplevel it creates, so this is present
/// for any toplevel that came through xdg-shell (i.e. all of them).
fn surface_of(toplevel: &XdgToplevel) -> Option<WlSurface> {
    toplevel.data::<XdgShellSurfaceUserData>().map(|d| d.wl_surface().clone())
}
