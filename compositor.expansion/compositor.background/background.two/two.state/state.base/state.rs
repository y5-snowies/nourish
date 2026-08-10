use compositor_background_two_draw_element::element::ParallaxBackground;

pub struct Two {
    pub instance: Option<ParallaxBackground>,
    /// This world's background-shader override (bundle name or absolute path);
    /// `None` falls back to the preference default, then the built-in parallax.
    /// Persisted per-world and rehydrated at world build by `BackgroundDoc`.
    pub background_shader: Option<String>,
    /// This world's edited shader-variable overrides, keyed by `@prop` name (so a
    /// value survives the shader's props being reordered/renamed). Empty = use the
    /// declared defaults. Persisted alongside `background_shader`.
    pub params: Vec<(String, f32)>,
    /// The selected shader's compile error for the active renderer (runtime only,
    /// not persisted); `None` when it compiled or the built-in is selected.
    pub shader_error: Option<String>,
    /// This world's baked pointer-warp map, for a bundle that declared
    /// `evaluate: "map_static"`. PER WORLD because the bundle is: a process-global
    /// cache could only ever hold one world's warp, and would hand the other
    /// world's pointer a displacement from a shader it is not running.
    ///
    /// Rebaked in place when the resolution moves (the only input that can, since
    /// `@prop`s are fixed at load). `None` until the first pointer event asks.
    pub warp_map: Option<compositor_pipeline_host_map_base::map::Map>,
    /// Per-world background pan inversion: flip the camera pan fed to the shader on
    /// each axis. Persisted per world; default off. Lets a world reverse its
    /// horizontal and/or vertical parallax without touching the shader source.
    pub invert_pan_x: bool,
    pub invert_pan_y: bool,
    /// Per-world sRGB output: when set, the background shader gamma-encodes its final
    /// colour so the non-sRGB scanout buffer shows the brighter, preview-matching
    /// look (default off = raw values). Persisted per world.
    pub srgb: bool,
    /// Per-world "Optimized" shader variant: render the built-in parallax with its
    /// cheap twin (`parallax_optimized.wgsl`) instead of the reference — same scene,
    /// far less value noise, for a GPU that cannot afford the real one. Default off,
    /// persisted per world. Only the built-in has an optimized variant today, so a
    /// world with a shader override ignores it (and the settings toggle is disabled).
    pub optimized: bool,
}

impl Two {
    /// This world's loaded multipass bundle, if it has one.
    pub fn bundle(
        &self,
    ) -> Option<&compositor_pipeline_build_pipeline_base::pipeline::CompiledPipeline> {
        self.instance.as_ref()?.pipeline.as_deref()
    }

    /// This world's editable variables, from the bundle that is actually running.
    ///
    /// `shader.load::properties_for` answers the same question from disk, and has
    /// to for a bundle that is merely listed rather than selected — but it re-reads
    /// and re-parses `pipeline.json` plus every pass source on each call, and it is
    /// called per param message and per frame while the panel is open. When a
    /// bundle IS loaded the answer is already in hand, free, and cannot disagree
    /// with what is on screen.
    pub fn props(&self) -> Option<&[compositor_pipeline_bundle_property_base::Property]> {
        Some(&self.bundle()?.properties)
    }

    /// Whether the engine should draw its own border around each window.
    ///
    /// The border is drawn OUTSIDE the slot and is opaque, so any effect reading
    /// window edges — a glow, a field between windows, a refraction — reads the
    /// border rather than the window under it. A bundle doing that suppresses it.
    /// `true` (draw it) whenever no bundle says otherwise, which is every world
    /// that is not running one.
    pub fn border(&self) -> bool {
        use compositor_pipeline_bundle_manifest_base::manifest::Decorations;
        self.bundle().map(|cp| cp.decorations != Decorations::Off).unwrap_or(true)
    }

    /// When to suppress the letterbox fill painted behind client content.
    pub fn letterbox(&self) -> compositor_pipeline_bundle_manifest_base::manifest::LetterboxMode {
        self.bundle().map(|cp| cp.letterbox).unwrap_or_default()
    }

    pub fn new() -> Self {
        Self {
            instance: None,
            background_shader: None,
            params: Vec::new(),
            shader_error: None,
            warp_map: None,
            invert_pan_x: false,
            invert_pan_y: false,
            srgb: false,
            optimized: false,
        }
    }
}
