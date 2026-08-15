use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;
use smithay::wayland::xdg_activation::{XdgActivationToken, XdgActivationTokenData};

#[derive(Clone)]
pub struct ActivationDetails {
    pub token: XdgActivationToken,
    pub token_data: XdgActivationTokenData,
}

// Pure surface-data write — does not touch `Dispatch` (so this crate stays a
// leaf, letting state.base host `impl XdgActivationHandler for Dispatch`
// without a dependency cycle — document/SMITHAY_DECOUPLING.md).
pub fn request_activation(
    surface: WlSurface,
    token: XdgActivationToken,
    token_data: XdgActivationTokenData,
) {
    with_states(&surface, |states| {
        let _inserted = states
            .data_map
            .insert_if_missing_threadsafe(|| ActivationDetails { token, token_data });
    });
}
