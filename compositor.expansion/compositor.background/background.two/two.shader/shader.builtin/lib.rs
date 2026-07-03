//! The built-in parallax shader's exposed `@prop` variables. The built-in has no
//! bundle file, so its property metadata lives here — used by `draw.select` to
//! seed defaults and by the settings UI to render its controls. The shaders
//! (`spacev3.frag` / `parallax.wgsl`) read these as `u_param0..` / push `params`:
//! slot 0 = drift speed, 1 = star density, 2 = nebula intensity, 3 = vignette
//! amount (0 = off), 4 = vignette radius (extent), 5 = vignette softness (feather).
//! The vignette is evaluated in screen space so it stays consistent across zoom.

use compositor_background_two_shader_property::{PropValue, Property};

/// The built-in shader's properties (slot = index), as if parsed from `@prop`.
pub fn builtin_props() -> Vec<Property> {
    let f = |name: &str, default: f32, min: f32, max: f32, label: &str| Property {
        name: name.to_string(),
        default: PropValue::Float(default),
        min: Some(min),
        max: Some(max),
        step: None,
        label: Some(label.to_string()),
        group: Some("Parallax".to_string()),
    };
    vec![
        f("drift_speed", 1.0, 0.0, 3.0, "Drift speed"),
        f("star_density", 1.0, 0.0, 2.0, "Star density"),
        f("nebula", 1.0, 0.0, 2.0, "Nebula intensity"),
        // Radius = where darkening reaches full at the edge; softness = how far it
        // feathers inward. Amount 0 = off.
        f("vignette", 0.0, 0.0, 1.0, "Vignette amount"),
        f("vignette_radius", 1.12, 0.5, 2.0, "Vignette radius"),
        f("vignette_softness", 0.6, 0.05, 2.0, "Vignette softness"),
    ]
}

/// The built-in shader's default params block.
pub fn default_params() -> [f32; 16] {
    compositor_background_two_shader_property::default_params(&builtin_props())
}
