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

use compositor_pipeline_bundle_property_base::{parse_props, PropValue, Property};

/// Built-in world selection ids carry this prefix so they never collide with a
/// user bundle folder name and are recognised without touching the disk.
pub const BUILTIN_PREFIX: &str = "builtin:";

/// One compiled-in built-in world: a stable selection id, the picker heading it
/// sits under, and its WGSL source.
pub struct Builtin {
    pub id: &'static str,
    /// The picker heading. The list below was already kept in themed runs marked
    /// by comments; naming the run makes that renderable instead of conventional.
    pub category: &'static str,
    pub wgsl: &'static str,
}

/// Heading for a bundle that names none: anything dropped into the shader folder
/// by hand. Findable without having declared anything.
pub const USER_CATEGORY: &str = "User";

/// Prefix on the heading of every bundle that came from the shader FOLDER, its
/// own declared category included.
///
/// A user bundle may name any heading it likes, `Multipass` among them, and the
/// picker groups by the heading STRING — so without this a dropped-in bundle
/// silently joins the curated set it happens to share a name with, and selecting
/// that heading shows both. Marking the origin keeps them two headings.
///
/// A word rather than a glyph on purpose: this crate says where the bundle came
/// from, and the settings rail decides what that looks like (it draws the mark as
/// a Material `Person` icon). A codepoint here would render as tofu anywhere the
/// icon font is not loaded.
pub const USER_MARK: &str = "user:";
/// Heading for the stock parallax, which is the default rather than a choice.
pub const DEFAULT_CATEGORY: &str = "Default";

/// The extra built-in worlds, in picker order: the orbital "galaxy" view, then
/// the "inside the world" surface scenes (drift / cave), then the standalone
/// underwater descent, then the calm nature/landscape/marine scenes, then the sky
/// / weather scenes (cloud drift, aurora, rain on glass, sunset birds), then the
/// fully abstract "wallpaper" set (metaballs, contours, voronoi) — smooth
/// low-contrast fields that never compete with the foreground. The stock space
/// parallax is NOT listed here — it stays the unnamed default (`None`).
///
/// Removing an entry is safe: a world that still names it falls back to the stock
/// parallax (see `REMOVED_BUILTINS` in `two.storage`, which rewrites the stale
/// selection to `None` on load so the settings panel stays truthful about it).
pub fn builtins() -> &'static [Builtin] {
    const SPACE: &str = "Space";
    const LAND: &str = "Landscape";
    const WATER: &str = "Water";
    const SKY: &str = "Sky";
    const ABSTRACT: &str = "Abstract";
    const UTILITY: &str = "Utility";
    &[
        Builtin { id: "builtin:fiery-galaxy", category: SPACE, wgsl: include_str!("shaders/fiery.wgsl") },
        // "Inside the world" surface scenes.
        Builtin { id: "builtin:leafy-drift", category: LAND, wgsl: include_str!("shaders/leafy_drift.wgsl") },
        Builtin { id: "builtin:rocky-cave", category: LAND, wgsl: include_str!("shaders/rocky_cave.wgsl") },
        Builtin { id: "builtin:misty-ridges", category: LAND, wgsl: include_str!("shaders/misty_ridges.wgsl") },
        Builtin { id: "builtin:dusk-dunes", category: LAND, wgsl: include_str!("shaders/dusk_dunes.wgsl") },
        Builtin { id: "builtin:firefly-meadow", category: LAND, wgsl: include_str!("shaders/firefly_meadow.wgsl") },
        Builtin { id: "builtin:papercut-layers", category: LAND, wgsl: include_str!("shaders/papercut.wgsl") },
        // Marine.
        Builtin { id: "builtin:underwater", category: WATER, wgsl: include_str!("shaders/underwater.wgsl") },
        Builtin { id: "builtin:ocean-horizon", category: WATER, wgsl: include_str!("shaders/ocean_horizon.wgsl") },
        Builtin { id: "builtin:harbor-beacon", category: WATER, wgsl: include_str!("shaders/harbor_beacon.wgsl") },
        Builtin { id: "builtin:harbor-piers", category: WATER, wgsl: include_str!("shaders/harbor_piers.wgsl") },
        // Sky / weather scenes.
        Builtin { id: "builtin:cloud-drift", category: SKY, wgsl: include_str!("shaders/cloud_drift.wgsl") },
        Builtin { id: "builtin:aurora", category: SKY, wgsl: include_str!("shaders/aurora.wgsl") },
        Builtin { id: "builtin:rain-glass", category: SKY, wgsl: include_str!("shaders/rain_glass.wgsl") },
        Builtin { id: "builtin:sunset-birds", category: SKY, wgsl: include_str!("shaders/sunset_birds.wgsl") },
        Builtin { id: "builtin:snowfall", category: SKY, wgsl: include_str!("shaders/snowfall.wgsl") },
        // Smooth low-contrast fields that never compete with the foreground.
        Builtin { id: "builtin:metaballs", category: ABSTRACT, wgsl: include_str!("shaders/metaballs.wgsl") },
        Builtin { id: "builtin:contours", category: ABSTRACT, wgsl: include_str!("shaders/contours.wgsl") },
        Builtin { id: "builtin:voronoi", category: ABSTRACT, wgsl: include_str!("shaders/voronoi.wgsl") },
        // Last deliberately: a measurement baseline, not a look. See black.wgsl.
        Builtin { id: "builtin:black", category: UTILITY, wgsl: include_str!("shaders/black.wgsl") },
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
        choices: Vec::new(),
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
    compositor_pipeline_bundle_property_base::default_params(&builtin_props())
}
