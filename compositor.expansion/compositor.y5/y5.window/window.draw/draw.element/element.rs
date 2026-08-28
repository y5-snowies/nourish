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
    // fallback path. Wrapped in `ClampOpaque`, which declines direct scanout when it spans
    // the output.
    Window = ClampOpaque<ElementWindowSurface<WaylandSurfaceRenderElement<R>>>,
    // Fitted window / popup surface: the toplevel content rescaled (aspect-fit), relocated
    // (centered in its slot), and cropped (to the slot, or to the output for popups). The
    // innermost `ElementWindowSurface` forces a fixed (scale-independent) geometry so the
    // result is correct regardless of the scale the active render path queries with (the
    // winit Vulkan path uses 1.0, the GLES damage tracker uses the output scale). Built on
    // smithay's element utils so geometry / src / damage transform correctly. The outermost
    // `ClampOpaque` reports opacity untouched and declines direct scanout when output-spanning.
    WindowFit = ClampOpaque<CropRenderElement<RelocateRenderElement<RescaleRenderElement<ElementWindowSurface<WaylandSurfaceRenderElement<R>>>>>>,
    SolidBox = SolidColorRenderElement,
}

/// Outermost window wrapper. Reports the client's opaque region **unchanged**, and
/// **declines direct scanout** for an element that spans the whole output. Everything
/// else (geometry, src, damage, draw, transform) is delegated untouched.
///
/// Why the scanout refusal: when a window's opaque region reaches the output edges *and*
/// its geometry spans the whole output, smithay's `DrmCompositor` treats it as a
/// fully-opaque output-spanning element, stops compositing it — culling everything below,
/// including the always-animating parallax background — and direct-scans its raw client
/// buffer onto the primary plane. For a window bigger than the monitor that wedges the
/// page-flip and freezes the display.
///
/// The name is now historical: this used to prevent that by CLAMPING the reported opaque
/// region to a centred 75% box, so it could never reach the output edges. See
/// [`Self::underlying_storage`] for why that was the wrong lever.
pub struct ClampOpaque<E> {
    pub inner: E,
    /// Screen / output size in physical pixels (the space `opaque_regions`/`geometry` report in).
    pub screen: Size<i32, Physical>,
    /// Whether the drawn world's bundle composites this window ITSELF
    /// (`worldset::Own::covers`).
    ///
    /// NOT occlusion. Nothing here knows or cares whether another window is
    /// stacked over this one; that question lives in `draw.occlude` and is spelt
    /// "covered" there. This says the engine is not the one drawing these pixels,
    /// so it may neither claim their opacity nor let them cull anything —
    /// `draw.frame::scene` is the other half, and the two must agree.
    ///
    /// Stamped at construction rather than read here: `opaque_regions` is a
    /// smithay trait method with no way to reach a world, and the value is a fact
    /// about the world whose band this window is in — which used to mean a
    /// process-global that any world could have written.
    pub bundle_owned: bool,
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
        // window too (`worldset::Claim::owns`) — leaving a hole exactly where it
        // moved from.
        if self.bundle_owned {
            return Default::default();
        }
        // The client's opaque region, UNTOUCHED. What used to happen here — clipping
        // it to a centred box so it could never reach the output edges — is now done
        // where it belongs, by declining direct scanout in `underlying_storage`.
        // See the type docs.
        self.inner.opaque_regions(scale)
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

    /// Decline DIRECT SCANOUT for an element that spans the whole output.
    ///
    /// This is the freeze guard, stated at the layer it belongs to. `underlying_storage`
    /// is how an element offers its raw client buffer for promotion onto a KMS plane;
    /// returning `None` says "composite me" and takes that path off the table. An
    /// output-spanning window is exactly the case that promotion goes wrong for: the
    /// buffer can be far larger than the display (zoom far enough into a window and it
    /// is), the plane cannot present it, and the page-flip wedges — taking the display
    /// with it, along with everything culled below it.
    ///
    /// This replaces the old approach of clipping the OPAQUE REGION to a centred box so
    /// it could never reach the output edges. That worked only by lying: opacity is a
    /// fact about pixels, scanout eligibility is a policy about planes, and stating the
    /// second by falsifying the first meant every window silently reported its border as
    /// non-opaque. Harmless while the renderer blended everything anyway — and the moment
    /// the renderer began honouring opacity, it surfaced as a see-through band that slid
    /// across each window as the camera panned. Saying the policy directly cannot leak
    /// into rendering that way.
    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage> {
        let geo = self.inner.geometry(Scale::from(1.0));
        let spans_output = geo.loc.x <= 0
            && geo.loc.y <= 0
            && geo.loc.x + geo.size.w >= self.screen.w
            && geo.loc.y + geo.size.h >= self.screen.h;
        if spans_output {
            return None;
        }
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
    /// See [`ClampOpaque::bundle_owned`] — the same fact, stamped for the same reason.
    pub bundle_owned: bool,
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
        if self.bundle_owned {
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
