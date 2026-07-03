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
}

impl Two {
    pub fn new() -> Self {
        Self { instance: None, background_shader: None, params: Vec::new(), shader_error: None }
    }
}
