pub mod wire {
    pub use compositor_support_smithay_state_compositor_session::{
        compositor_state, client_compositor_state, commit, apply_commit, awaits_initial_configure, toplevel_window,
    };
    pub use compositor_support_smithay_state_compositor_place::WindowPlacedMarker;
    pub use compositor_support_smithay_state_compositor_place::handle_commit;
}
