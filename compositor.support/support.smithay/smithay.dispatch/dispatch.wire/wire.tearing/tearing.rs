//! `wp_tearing_control_v1` — the client-side pacer tag.
//!
//! The protocol carries NO synchronization of its own; `set_presentation_hint`
//! is purely a statement of preference that a compositor may ignore.
//!
//! It writes the CLIENT's half of `pacer::TearingTag` — "do I want to tear" — and only
//! that. The compositor's own heuristic verdict is the other half of the same tag and
//! is never touched from here.
//!
//! Whether a surface's flips actually tear remains the compositor's decision
//! (`y5.graphic/graphic.tearing`), but the client's statement outranks the heuristic
//! wherever it exists: an explicit `vsync` un-tags the window for both sections.
//!
//! The global is advertised unconditionally — a client may announce its intent
//! before the user has picked a mode, and revoking a bound global is not
//! something the protocol allows.
//!
//! The `Dispatch`/`GlobalDispatch` impls live in `dispatch.state/state.base`
//! alongside the other hand-rolled protocols, since that is where
//! `delegate_dispatch2!(Dispatch)` makes the bounds provable.

use compositor_support_smithay_state_tearing_pacer::pacer::TearingTag;
use smithay::reexports::wayland_protocols::wp::tearing_control::v1::server::{
    wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
    wp_tearing_control_v1::{self, PresentationHint, WpTearingControlV1},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DataInit, Dispatch, DisplayHandle, GlobalDispatch};
use smithay::wayland::compositor::with_states;

pub fn create_global<W>(dh: &DisplayHandle)
where
    W: GlobalDispatch<WpTearingControlManagerV1, ()> + 'static,
{
    dh.create_global::<W, WpTearingControlManagerV1, ()>(1, ());
    info!("tearing: wp_tearing_control_manager_v1 global advertised");
}

/// `get_tearing_control` hands the child object the surface as its user data, so
/// the hint can be routed back without a side table.
pub fn dispatch_manager<D>(
    request: wp_tearing_control_manager_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<WpTearingControlV1, WlSurface> + 'static,
{
    if let wp_tearing_control_manager_v1::Request::GetTearingControl { id, surface } = request {
        info!("tearing: client took a tearing control for a surface");
        di.init(id, surface);
    }
}

/// Apply a hint to its surface.
///
/// Writes ONLY the client's half of `pacer::TearingTag` (`set_hint`). The compositor's
/// heuristic verdict is the other half, and a client answering about its own flips must
/// not erase it — which is what one shared flag used to do on every hint.
pub fn dispatch_control(request: wp_tearing_control_v1::Request, surface: &WlSurface) {
    let wp_tearing_control_v1::Request::SetPresentationHint { hint } = request else {
        return;
    };
    let wants_tearing = matches!(hint.into_result(), Ok(PresentationHint::Async));
    info!(
        "tearing: presentation hint = {} (surface tagged as pacer: {wants_tearing})",
        if wants_tearing { "async" } else { "vsync" }
    );
    with_states(surface, |states| {
        states.data_map.insert_if_missing(TearingTag::default);
        if let Some(tag) = states.data_map.get::<TearingTag>() {
            tag.set_hint(wants_tearing);
        }
    });
}
