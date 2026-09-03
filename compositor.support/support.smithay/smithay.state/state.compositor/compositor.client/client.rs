//! The per-client compositor state smithay hands back on every commit.
//!
//! Split out from the commit path because it answers a question about the CLIENT
//! record, not about a surface: which `ClientData` is on this connection, and where
//! in it the `CompositorClientState` lives. Two kinds of client reach it, and only
//! one of them is ours.

use smithay::reexports::wayland_server::Client;
use smithay::wayland::compositor::CompositorClientState;
use compositor_support_smithay_wayland_connection_record::record::WaylandClientSession;

/// Every ordinary client carries the session record the socket source attaches when
/// it connects. The Xwayland server does not: smithay spawns it and inserts its own
/// `XWaylandClientData` (which is where the per-client scale it needs lives), so it
/// arrives here with no y5 session at all. A client with neither is a client we did
/// not create and smithay did not either — there is no third source, so it aborts.
pub fn client_compositor_state(client: &Client) -> &CompositorClientState {
    if let Some(session) = client.get_data::<WaylandClientSession>() {
        return &session.compositor_state;
    }
    client
        .get_data::<smithay::xwayland::XWaylandClientData>()
        .map(|data| &data.compositor_state)
        .unwrap_or_else(|| abort!("client has neither a y5 session nor XWayland client data"))
}
