//! The built-in background shaders.
//!
//! Two things live here. First, the stock parallax shader's exposed `@prop`
//! variables: the stock built-in has no bundle file, so its property metadata is
//! hardcoded in [`builtin_props`] — used by `draw.select` to seed defaults and by
//! the settings UI to render its controls. The shaders (`spacev3.frag` /
//! `parallax.wgsl`) read these as `u_param0..` / push `params`: slot 0 = drift
//! speed, 1 = star density, 2 = nebula intensity, 3 = vignette amount (0 = off),
//! 4 = vignette radius (extent), 5 = vignette softness (feather). The vignette is
//! evaluated in screen space so it stays consistent across zoom.
//!
//! Second, the extra built-in *worlds* that ship compiled into the binary (their
//! WGSL is `include_str!`'d here) and appear in the shader picker alongside the
//! stock parallax. They are resolved from their
//! `builtin:` selection id straight to source, with no disk access, and compiled
//! through the same runtime WGSL path as user bundles (see `shader.load`); their
//! `@prop` controls are parsed from the source, so there is nothing to duplicate.

pub mod base;
pub use base::*;
