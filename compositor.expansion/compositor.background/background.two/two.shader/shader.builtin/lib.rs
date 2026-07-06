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
//! Second, the extra built-in *worlds* — leafy / rocky / fiery — that ship
//! compiled into the binary (their WGSL is `include_str!`'d here) and appear in
//! the shader picker alongside the stock parallax. They are resolved from their
//! `builtin:` selection id straight to source, with no disk access, and compiled
//! through the same runtime WGSL path as user bundles (see `shader.load`); their
//! `@prop` controls are parsed from the source, so there is nothing to duplicate.

use compositor_background_two_shader_property::{parse_props, PropValue, Property};

/// Built-in world selection ids carry this prefix so they never collide with a
/// user bundle folder name and are recognised without touching the disk.
pub const BUILTIN_PREFIX: &str = "builtin:";

/// One compiled-in built-in world: a stable selection id and its WGSL source.
pub struct Builtin {
    pub id: &'static str,
    pub wgsl: &'static str,
}

/// The extra built-in worlds, in picker order: the three orbital "galaxy" views
/// first, then the three "inside the world" surface scenes. The stock space
/// parallax is NOT listed here — it stays the unnamed default (`None` selection).
pub fn builtins() -> &'static [Builtin] {
    &[
        Builtin { id: "builtin:leafy-galaxy", wgsl: include_str!("shaders/leafy.wgsl") },
        Builtin { id: "builtin:rocky-galaxy", wgsl: include_str!("shaders/rocky.wgsl") },
        Builtin { id: "builtin:fiery-galaxy", wgsl: include_str!("shaders/fiery.wgsl") },
        Builtin { id: "builtin:leafy-drift", wgsl: include_str!("shaders/leafy_drift.wgsl") },
        Builtin { id: "builtin:rocky-cave", wgsl: include_str!("shaders/rocky_cave.wgsl") },
        Builtin { id: "builtin:fiery-cavern", wgsl: include_str!("shaders/fiery_cavern.wgsl") },
    ]
}

/// The WGSL source for a built-in selection id, if `id` names one.
pub fn source(id: &str) -> Option<&'static str> {
    builtins().iter().find(|b| b.id == id).map(|b| b.wgsl)
}

/// The `@prop` schema for a built-in selection id, parsed from its WGSL source.
pub fn props(id: &str) -> Option<Vec<Property>> {
    source(id).map(parse_props)
}

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
