pub mod state {
    pub use compositor_support_smithay_state_fractional_scale::{
        Fractional, FractionalScaleConfig, DebounceCycle, Published,
    };
    pub use compositor_support_smithay_state_fractional_emit::{
        emit_to_surfaces, NestedCompositorSurface,
    };
}
