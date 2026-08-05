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

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// Protocol version we advertise. Staging is at v1 in both namespaces.
pub const VERSION: u32 = 1;

/// User data of a bound `xdg_session_v1`: the id this session resolved to.
#[derive(Debug)]
pub struct SessionData {
    pub session_id: String,
}

/// User data of a per-toplevel session handle, in either namespace. The name is
/// mutable because `xdg_`'s `rename` re-keys the toplevel while keeping its
/// state; the `xx_` path never touches it. `toplevel` is kept for `xx_`, whose
/// `restored` event carries the object back to the client.
#[derive(Debug)]
pub struct ToplevelSessionData {
    pub session_id: String,
    pub name: std::sync::Mutex<String>,
    pub surface: Option<WlSurface>,
    pub toplevel: Option<XdgToplevel>,
}

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
    // Posted on the manager, not the new object: the `xdg_session_v1` is still
    // an uninitialised `New<_>` at this point and has no resource to error on.
    // The error kills the client, so leaving the id uninitialised is moot.
    let Ok(reason) = reason.into_result() else {
        manager.post_error(
            xdg_session_manager_v1::Error::InvalidReason,
            "unknown xdg_session_manager_v1 reason",
        );
        return;
    };

    match session_id.filter(|s| store.exists(s)) {
        // Known id — the client is coming back. Its toplevels can now restore
        // by name, which is the entire point of the protocol.
        Some(existing) => {
            let session = di.init(id, SessionData { session_id: existing.clone() });
            session.restored();
            info!("session: restored session {existing} (reason {reason:?})");
        }
        // New (or unrecognised — the protocol says treat that as NULL) session.
        // When we can work out which placeholder launched this client, mint the
        // id THAT placeholder already restores under (falling back to its uuid
        // if it has never seen one), so a client that asks with NULL on every
        // run still gets a stable string back and can find its own state.
        // Otherwise a fresh id, still durable for later runs.
        None => {
            let pid = client.get_credentials(dh).map(|c| c.pid).unwrap_or(-1);
            let minted = compositor_support_smithay_state_session_claim::claim::resolve(pid)
                .unwrap_or_else(|| Uuid::now_v7().to_string());
            store.open(&minted);
            let session = di.init(id, SessionData { session_id: minted.clone() });
            session.created(minted.clone());
            info!("session: minted session {minted} for pid {pid} (reason {reason:?})");
        }
    }
}

/// `add_toplevel` / `restore_toplevel` / `remove_toplevel`.
///
/// Both add and restore stamp the identity onto the toplevel's surface data —
/// that write is what the placeholder matcher reads. They differ only in the
/// `restored` event, which restore emits when the name was already known
/// (before the initial configure, as the protocol requires, because the client
/// must issue the request before its first commit).
pub fn dispatch_session<D>(
    store: &mut SessionStore,
    session_id: &str,
    request: xdg_session_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XdgToplevelSessionV1, ToplevelSessionData> + 'static,
{
    match request {
        xdg_session_v1::Request::AddToplevel { id, toplevel, name } => {
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            store.remember(session_id, &name);
            di.init(id, data(session_id, &name, surface, Some(toplevel)));
        }
        xdg_session_v1::Request::RestoreToplevel { id, toplevel, name } => {
            let known = store.knows(session_id, &name);
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            store.remember(session_id, &name);
            let handle = di.init(id, data(session_id, &name, surface, Some(toplevel)));
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
    store.forget(&data.session_id, &current);
    store.remember(&data.session_id, &name);
    stamp(data.surface.as_ref(), &data.session_id, &name);
    *current = name;
}

// ── xx_ namespace (GTK 4.22) ─────────────────────────────────────────────────
// Same store, same claim registry, different wire shape — see `mod legacy`.

pub fn create_legacy_global<D>(dh: &DisplayHandle)
where
    D: GlobalDispatch<XxSessionManagerV1, ()> + 'static,
{
    dh.create_global::<D, XxSessionManagerV1, ()>(VERSION, ());
    info!("session: xx_session_manager_v1 global advertised (GTK 4.22 namespace)");
}

/// `xx_session_manager_v1.get_session`. Mirrors [`dispatch_manager`], except the
/// `xx_` error enum has no `invalid_reason`, so an unparseable reason is logged
/// and treated as `launch` rather than killing the client.
pub fn dispatch_legacy_manager<D>(
    store: &mut SessionStore,
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
    match session.filter(|s| store.exists(s)) {
        Some(existing) => {
            let s = di.init(id, SessionData { session_id: existing.clone() });
            s.restored();
            info!("session(xx): restored session {existing}");
        }
        None => {
            let pid = client.get_credentials(dh).map(|c| c.pid).unwrap_or(-1);
            let minted = compositor_support_smithay_state_session_claim::claim::resolve(pid)
                .unwrap_or_else(|| Uuid::now_v7().to_string());
            store.open(&minted);
            let s = di.init(id, SessionData { session_id: minted.clone() });
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
    session_id: &str,
    request: xx_session_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XxToplevelSessionV1, ToplevelSessionData> + 'static,
{
    match request {
        xx_session_v1::Request::AddToplevel { id, toplevel, name } => {
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            store.remember(session_id, &name);
            di.init(id, data(session_id, &name, surface, Some(toplevel)));
        }
        xx_session_v1::Request::RestoreToplevel { id, toplevel, name } => {
            let known = store.knows(session_id, &name);
            let surface = surface_of(&toplevel);
            stamp(surface.as_ref(), session_id, &name);
            store.remember(session_id, &name);
            let handle = di.init(id, data(session_id, &name, surface, Some(toplevel.clone())));
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

fn data(
    session_id: &str,
    name: &str,
    surface: Option<WlSurface>,
    toplevel: Option<XdgToplevel>,
) -> ToplevelSessionData {
    ToplevelSessionData {
        session_id: session_id.to_string(),
        name: std::sync::Mutex::new(name.to_string()),
        surface,
        toplevel,
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
fn surface_of(
    toplevel: &smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel,
) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
    toplevel.data::<XdgShellSurfaceUserData>().map(|d| d.wl_surface().clone())
}
