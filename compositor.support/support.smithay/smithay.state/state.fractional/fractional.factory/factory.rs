use smithay::reexports::wayland_protocols::wp::fractional_scale::v1::server::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};

use smithay::reexports::wayland_server::{Dispatch, DisplayHandle, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::fractional_scale::{
    FractionalScaleData, FractionalScaleHandler, FractionalScaleManagerState, FractionalScaleState,
};
use smithay::wayland::presentation::{PresentationData, PresentationState};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_fractional_base::state::{Fractional, FractionalScaleConfig};

pub fn new<I: DispatchWire>(display_handle: &DisplayHandle) -> Fractional
where
    I: GlobalDispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
        + Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
        + Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, FractionalScaleData>
        + 'static,
    I: FractionalScaleHandler + 'static {
    let mut fractional_manager_state = FractionalScaleManagerState::new::<I>(&display_handle);
    // NOT advertised to Xwayland.
    //
    // `wp_fractional_scale_v1` asks a client to render at a scale the compositor picks.
    // Xwayland binds it and applies the answer to X windows, which never asked and cannot
    // be asked — so the scale becomes a negotiation between y5 and a proxy about windows
    // neither owns. Hidden, Xwayland falls back to the integer
    // `wl_surface.set_buffer_scale` path, which is what xwayland-satellite was deployed
    // with (`--force-scale 1 --ignore-fractional-scale`).
    smithay::wayland::fractional_scale::set_visibility_filter(|client| {
        !compositor_support_smithay_state_xwayland_base::base::is_xwayland(client)
    });
    let cfg = FractionalScaleConfig::default();
    // Seed the published scale rather than starting at "unknown".
    //
    // `last_emitted_scale` is what a brand-new surface or scale object is handed before any
    // render pass has run, and it was only ever assigned from `emit_best_per_surface` — so
    // until the first window had been drawn it was `None` and the answer was to send
    // nothing at all. A client then laid its FIRST buffer out at scale 1, and only learnt
    // the real scale afterwards; anything that does not spontaneously repaint (or whose
    // size the compositor has since frozen) stayed wrong until something forced it to
    // redraw. Under v2 that is worse than blurry, because surface-local geometry is
    // expressed in the declared space: the whole subsurface layout comes out at the wrong
    // size and stays there.
    //
    // The floor is the right seed. A window being mapped is on screen by definition, so it
    // gets at least the lattice minimum instead of 1.0, and the first real render pass
    // replaces it with the window's own value.
    return Fractional {
        state: fractional_manager_state,
        last_emitted_scale: Some(cfg.min_scale),
        cfg,
        last_observed: None,
        cycle: None,
        armed: false,
        last_emit_at: None,
    };
}
