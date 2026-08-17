//! Per-surface HDR color tag, shared between the `wp_color_management_v1`
//! protocol (writer, in the support layer) and the renderer (reader, in the
//! backend). A small leaf type stored in the `wl_surface`'s state so neither
//! layer has to depend on the other.

pub mod base;
pub use base::*;
