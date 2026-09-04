pub mod wire {
    pub use compositor_support_smithay_state_compositor_session::{
        compositor_state, commit, apply_commit, awaits_initial_configure,
    };
    pub use compositor_support_smithay_state_compositor_client::client::client_compositor_state;
    pub use compositor_support_smithay_state_compositor_place::WindowPlacedMarker;
    pub use compositor_support_smithay_state_compositor_place::handle_commit;
}
