//! Facade: `ParallaxBackground` lives in the flat sibling crates
//! (`draw.parallax` / `draw.motion` / `draw.program`); the public
//! `element::ParallaxBackground` path keeps resolving. The `shaders/`
//! directory stays here (`draw.program` embeds `shaders/spacev3.frag`).

pub mod element {
    pub use compositor_background_two_draw_parallax::ParallaxBackground;
    /// Pane identity, re-exported on the same facade. The scene builder no longer
    /// derives a key of its own — it hands `bind_pane` the output and the region
    /// index and the element builds one, so there is a single derivation.
    pub use compositor_background_two_draw_parallax::{PaneKey, Region};
}
