use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{
    CropRenderElement, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::{
    Element as ElementSmithay, Id, RenderElement, UnderlyingStorage,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportMem};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size};

smithay::render_elements! {
    pub Element<R> where R: ImportAll + ImportMem;
    // Native window surface (camera-zoom only) — used for the client-driven (no decided size)
    // fallback path. Wrapped in `ClampOpaque` so its reported opacity can never span the output.
    Window = ClampOpaque<ElementWindowSurface<WaylandSurfaceRenderElement<R>>>,
    // Fitted window / popup surface: the toplevel content rescaled (aspect-fit), relocated
    // (centered in its slot), and cropped (to the slot, or to the output for popups). The
    // innermost `ElementWindowSurface` forces a fixed (scale-independent) geometry so the
    // result is correct regardless of the scale the active render path queries with (the
    // winit Vulkan path uses 1.0, the GLES damage tracker uses the output scale). Built on
    // smithay's element utils so geometry / src / damage transform correctly. The outermost
    // `ClampOpaque` clamps the reported opaque region to the central 75% of the screen.
    WindowFit = ClampOpaque<CropRenderElement<RelocateRenderElement<RescaleRenderElement<ElementWindowSurface<WaylandSurfaceRenderElement<R>>>>>>,
    SolidBox = SolidColorRenderElement,
}

/// Fraction of the screen the reported opaque region is allowed to cover (centered).
const OPAQUE_CLAMP_FRACTION: f64 = 0.75;

/// Outermost wrapper that **clamps a window's reported opaque region to the central
/// `OPAQUE_CLAMP_FRACTION` of the screen** (centered on the output). Everything else
/// (geometry, src, damage, draw, scan-out, transform) is delegated untouched — only the
/// *opaque* regions are shrunk.
///
/// Why: when a window's opaque region reaches the output edges *and* its geometry spans the
/// whole output, smithay's `DrmCompositor` treats it as a fully-opaque output-spanning element
/// and stops compositing it — culling everything below (incl. the always-animating parallax
/// background) and direct-scanning its raw client buffer onto the primary plane. For a window
/// bigger than the monitor that wedges the page-flip and freezes the display. Guaranteeing a
/// ≥`(1-fraction)/2` margin on every screen edge means the opaque region can never cover the
/// whole output, so that path never triggers — while occlusion culling still works for the
/// (large) central region.
pub struct ClampOpaque<E> {
    pub inner: E,
    /// Screen / output size in physical pixels (the space `opaque_regions`/`geometry` report in).
    pub screen: Size<i32, Physical>,
    /// Whether the drawn world's bundle composites this window itself.
    ///
    /// Stamped at construction rather than read here: `opaque_regions` is a
    /// smithay trait method with no way to reach a world, and the value is a fact
    /// about the world whose band this window is in — which used to mean a
    /// process-global that any world could have written.
    pub covered: bool,
}

impl<E: ElementSmithay> ElementSmithay for ClampOpaque<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn transform(&self) -> smithay::utils::Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> smithay::backend::renderer::utils::DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(
        &self,
        scale: Scale<f64>,
    ) -> smithay::backend::renderer::utils::OpaqueRegions<i32, Physical> {
        // A bundle that draws this window itself may put it anywhere, so it no
        // longer covers what is behind it. Claiming opacity here would subtract the
        // background band's damage under the window, and the engine then skips the
        // window too (`Claim::owns_op`) — leaving a hole exactly where it moved from.
        if self.covered {
            return Default::default();
        }
        // Central `OPAQUE_CLAMP_FRACTION` of the screen, in absolute output-physical coords.
        let cw = (self.screen.w as f64 * OPAQUE_CLAMP_FRACTION).round() as i32;
        let ch = (self.screen.h as f64 * OPAQUE_CLAMP_FRACTION).round() as i32;
        let clamp = Rectangle::new(
            Point::from(((self.screen.w - cw) / 2, (self.screen.h - ch) / 2)),
            Size::from((cw, ch)),
        );
        // smithay reports opaque regions *relative to the element* (its geometry origin).
        // Lift each into absolute output space, clip to the clamp box, then put back.
        let origin = self.inner.geometry(scale).loc;
        self.inner
            .opaque_regions(scale)
            .into_iter()
            .filter_map(|mut r| {
                r.loc += origin;
                let mut c = r.intersection(clamp)?;
                c.loc -= origin;
                Some(c)
            })
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> smithay::backend::renderer::element::Kind {
        self.inner.kind()
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.inner.is_framebuffer_effect()
    }
}

impl<R, E> RenderElement<R> for ClampOpaque<E>
where
    R: smithay::backend::renderer::Renderer,
    E: RenderElement<R>,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner.draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage> {
        self.inner.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut <R>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), <R>::Error> {
        self.inner.capture_framebuffer(frame, src, dst, cache)
    }
}

/// A surface render element whose geometry/location are forced to use a fixed `zoom` scale
/// instead of the damage tracker's output scale. Used to inject the y5 camera zoom into the
/// native (un-fitted) render path. (The fitted path uses smithay's `RescaleRenderElement`.)
pub struct ElementWindowSurface<E> {
    pub inner: E,
    pub zoom: f64,
    /// See [`ClampOpaque::covered`] — the same fact, stamped for the same reason.
    pub covered: bool,
}

impl<E: ElementSmithay> ElementSmithay for ElementWindowSurface<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(Scale::from(self.zoom))
    }

    fn location(&self, _scale: Scale<f64>) -> smithay::utils::Point<i32, Physical> {
        self.inner.location(Scale::from(self.zoom))
    }

    fn transform(&self) -> smithay::utils::Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> smithay::backend::renderer::utils::DamageSet<i32, Physical> {
        self.inner.damage_since(Scale::from(self.zoom), commit)
    }

    fn opaque_regions(
        &self,
        _scale: Scale<f64>,
    ) -> smithay::backend::renderer::utils::OpaqueRegions<i32, Physical> {
        if self.covered {
            return Default::default();
        }
        self.inner.opaque_regions(Scale::from(self.zoom))
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> smithay::backend::renderer::element::Kind {
        self.inner.kind()
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.inner.is_framebuffer_effect()
    }
}

impl<R, E> RenderElement<R> for ElementWindowSurface<E>
where
    R: smithay::backend::renderer::Renderer,
    E: RenderElement<R>,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner.draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage> {
        self.inner.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut <R>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), <R>::Error> {
        self.inner.capture_framebuffer(frame, src, dst, cache)
    }
}
