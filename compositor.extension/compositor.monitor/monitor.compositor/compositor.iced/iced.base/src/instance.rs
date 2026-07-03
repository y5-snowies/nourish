//! Per-instance bundle: `IcedSurface` + `IcedRuntime<U>` + placement + space.
//!
//! `IcedInstance<U>`: concrete, knows `U::Message`.
//! `IcedItem`: type-erased newtype the registry stores and yields.
//! `IcedInstanceAny`: private vtable behind `IcedItem`.
//!
//! The item's `location` is in its own coordinate space (World coords for
//! `IcedSpace::World`, screen physical pixels for `IcedSpace::Screen`).
//! The registry and the compositor decide how to project this to screen
//! coords at render time using a camera `Transform` and output size,
//! passed as arguments. This mirrors how Smithay's `window.render_elements`
//! takes the screen location and zoom as arguments rather than storing them.

use std::any::{Any, TypeId};

use iced_core::{Event as IcedEvent, mouse};
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_support_iced_core_engine_base::{IcedRuntime, IcedUi};
use compositor_monitor_runtime_surface_base::{IcedSurface, SurfaceError, WgpuVulkanContext};

use crate::element::IcedRenderElement;
use crate::handle::HandleId;
use crate::space::{IcedSpace, Transform};

// ── Concrete instance ──────────────────────────────────────────────────

pub struct IcedInstance<U: IcedUi> {
    pub(crate) id: HandleId,
    pub(crate) smithay_id: Id,
    pub(crate) commit: CommitCounter,
    pub(crate) location: Point<i32, Physical>,
    pub(crate) surface: IcedSurface,
    pub(crate) scale_factor: f32,

    pub(crate) runtime: IcedRuntime<U>,
    pub(crate) pending_resize: Option<(Size<i32, Physical>, f32)>,
}

impl<U: IcedUi> IcedInstance<U> {
    pub fn handle_id(&self) -> HandleId {
        self.id
    }
    pub fn location(&self) -> Point<i32, Physical> {
        self.location
    }
    pub fn size(&self) -> Size<i32, Physical> {
        self.surface.size
    }
    pub fn runtime(&self) -> &IcedRuntime<U> {
        &self.runtime
    }
    pub fn runtime_mut(&mut self) -> &mut IcedRuntime<U> {
        &mut self.runtime
    }
    pub fn ui(&self) -> &U {
        &self.runtime.ui
    }
    pub fn ui_mut(&mut self) -> &mut U {
        &mut self.runtime.ui
    }
}

// ── Vtable ─────────────────────────────────────────────────────────────

pub(crate) trait IcedInstanceAny: Any {
    fn handle_id(&self) -> HandleId;
    fn smithay_id(&self) -> &Id;
    fn commit(&self) -> CommitCounter;
    fn bump_commit(&mut self);
    fn location(&self) -> Point<i32, Physical>;
    fn size(&self) -> Size<i32, Physical>;
    /// The iced viewport scale factor (logical = physical / scale_factor). For
    /// `World` items this is the zoom counter-scale (`1/zoom`); for `Screen`
    /// items it's the instance scale (currently always 1.0).
    fn scale_factor(&self) -> f32;
    fn set_location(&mut self, p: Point<i32, Physical>);
    fn queue_event(&mut self, event: IcedEvent);
    fn tick(&mut self) -> bool;
    /// True if the runtime still wants to be rendered next frame — dirty or
    /// mid-animation. Drives the host's "keep scheduling frames" decision.
    fn wants_frame(&self) -> bool;
    fn render(&mut self);
    fn texture_handle(&self) -> &smithay::backend::renderer::gles::GlesTexture;
    /// Strict read-only accessor for the surface's underlying dmabuf, so a
    /// non-GLES renderer (Vulkan) can import the iced output natively. (iced
    /// renders via wgpu-Vulkan into this dmabuf; GLES samples the imported
    /// texture above. Native Vulkan iced output supersedes this later.)
    fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf;
    fn apply_pending_resize(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
    ) -> Result<bool, SurfaceError>;
    fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32);
    fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)>;
    fn pointer_leave(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<U: IcedUi> IcedInstanceAny for IcedInstance<U> {
    fn handle_id(&self) -> HandleId {
        self.id
    }
    fn smithay_id(&self) -> &Id {
        &self.smithay_id
    }
    fn commit(&self) -> CommitCounter {
        self.commit
    }
    fn bump_commit(&mut self) {
        self.commit.increment();
    }
    fn location(&self) -> Point<i32, Physical> {
        self.location
    }
    fn size(&self) -> Size<i32, Physical> {
        self.surface.size
    }
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
    fn set_location(&mut self, p: Point<i32, Physical>) {
        if self.location != p {
            self.location = p;
            // Mark changed so the move is picked up (damage), not rendered stale.
            self.commit.increment();
        }
    }
    fn queue_event(&mut self, event: IcedEvent) {
        self.runtime.queue_event(event);
    }
    fn tick(&mut self) -> bool {
        let changed = self.runtime.tick();
        if changed {
            trace!("tick changed handle={:?}", self.id);
        }
        changed
    }
    fn wants_frame(&self) -> bool {
        self.runtime.is_dirty()
    }
    fn render(&mut self) {
        let view = self.surface.create_render_view();
        self.runtime.render_into(&view);
        self.commit.increment();
    }
    fn texture_handle(&self) -> &smithay::backend::renderer::gles::GlesTexture {
        &self.surface.gles_texture
    }
    fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        &self.surface.allocated.dmabuf
    }
    fn apply_pending_resize(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
    ) -> Result<bool, SurfaceError> {
        let Some((new_size, scale_factor)) = self.pending_resize.take() else {
            return Ok(false);
        };
        if new_size == self.surface.size && scale_factor == self.scale_factor {
            return Ok(false);
        }
        trace!("resize handle={:?} old={:?} new={new_size:?}", self.id, self.surface.size);

        self.surface.resize(render_node, wgpu_ctx, gles, new_size)?;

        // println!("Scale factor just updated.");
        self.scale_factor = scale_factor;

        self.runtime
            .resize((new_size.w as u32, new_size.h as u32), self.scale_factor);
        self.commit.increment();
        Ok(true)
    }
    fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32) {
        // runtime doesnt keep track of instance scale. its safe to add
        self.pending_resize = Some((new_size, scale_factor));
    }
    fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)> {
        self.pending_resize
    }
    fn pointer_leave(&mut self) {
        self.runtime
            .queue_event(IcedEvent::Mouse(mouse::Event::CursorLeft));
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── IcedItem (the registry's stored type) ─────────────────────────────

pub struct IcedItem {
    pub inner: Box<dyn IcedInstanceAny>,
    type_id: TypeId,
    space: IcedSpace,
    pub layer: u64,
    /// When false the item is skipped by `elements`/`element_of` and never
    /// hit-tested. Lets a surface (e.g. a tooltip) be toggled on/off without
    /// reallocating its texture.
    visible: bool,
    /// When true the item is excluded from `hit_test`, so it floats above
    /// without intercepting pointer input or stealing events from what's
    /// behind it. Tooltips set this.
    passthrough: bool,
    /// Optional owning physical output (an opaque caller-supplied tag, e.g. an
    /// `output_key`). `None` = the surface is not bound to a specific monitor
    /// and follows the compositor's default single-output placement (launcher,
    /// dialogs, cursor). `Some(tag)` = the surface belongs to exactly that
    /// output; the scene builder draws it only on that monitor and the pointer
    /// path hit-tests it only when the cursor is on that monitor. Lets a screen
    /// overlay (e.g. a per-monitor capture stop button) be replicated once per
    /// output without duplicating/mispositioning on the others.
    output: Option<String>,
}

impl IcedItem {
    pub(crate) fn new<U: IcedUi>(inst: IcedInstance<U>, space: IcedSpace, layer: u64) -> Self {
        Self {
            type_id: TypeId::of::<IcedInstance<U>>(),
            inner: Box::new(inst),
            space,
            layer,
            visible: true,
            passthrough: false,
            output: None,
        }
    }

    // ── Visibility & passthrough ──────────────────────────────────

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Show/hide without destroying. Bumps the commit counter so smithay
    /// damages the item's rect on the transition.
    pub fn set_visible(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            self.inner.bump_commit();
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.passthrough
    }

    pub fn set_passthrough(&mut self, passthrough: bool) {
        self.passthrough = passthrough;
    }

    /// The output this surface is bound to, if any (see the `output` field).
    pub fn output(&self) -> Option<&str> {
        self.output.as_deref()
    }

    /// Bind (or unbind, with `None`) this surface to a specific physical output.
    pub fn set_output(&mut self, output: Option<String>) {
        self.output = output;
    }

    /// True if the runtime still wants to be rendered next frame (dirty or
    /// mid-animation).
    pub fn wants_frame(&self) -> bool {
        self.inner.wants_frame()
    }

    // ── Identity ──────────────────────────────────────────────────

    pub fn handle_id(&self) -> HandleId {
        self.inner.handle_id()
    }
    pub fn space(&self) -> IcedSpace {
        self.space
    }

    /// Change the item's space at runtime. Bumps the commit counter so
    /// the old screen rect is damaged.
    pub fn set_space(&mut self, space: IcedSpace) {
        if self.space != space {
            self.space = space;
            self.inner.bump_commit();
        }
    }

    // ── Stored geometry (in the item's own space) ────────────────

    /// Position in the item's own space (World coords if `space() == World`;
    /// physical screen pixels if `space() == Screen`).
    pub fn location(&self) -> Point<i32, Physical> {
        self.inner.location()
    }

    /// Natural (unscaled) size in physical pixels.
    pub fn size(&self) -> Size<i32, Physical> {
        self.inner.size()
    }

    pub fn set_location(&mut self, p: Point<i32, Physical>) {
        self.inner.set_location(p);
    }

    // ── Camera-aware geometry (passed transform + output size) ─────

    /// Compute this item's on-screen position given the camera transform
    /// and output size. For `Screen` items, returns the stored location
    /// unchanged; for `World` items, applies the transform.
    pub fn screen_location(
        &self,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Point<i32, Physical> {
        match self.space {
            IcedSpace::Screen => self.inner.location(),
            IcedSpace::World => {
                let world = self.inner.location();
                let s = transform
                    .world_to_screen(output_size, Point::from((world.x as f64, world.y as f64)));
                Point::from((s.x as i32, s.y as i32))
            }
        }
    }

    /// On-screen size, scaled by zoom for World items.
    pub fn screen_size(&self, transform: &Transform) -> Size<i32, Physical> {
        let size = self.inner.size();
        match self.space {
            IcedSpace::Screen => size,
            IcedSpace::World => Size::from((
                (size.w as f64 * transform.zoom) as i32,
                (size.h as f64 * transform.zoom) as i32,
            )),
        }
    }

    /// Full on-screen rectangle (location + size, both transformed).
    pub fn screen_rect(
        &self,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Rectangle<i32, Physical> {
        Rectangle::from_loc_and_size(
            self.screen_location(transform, output_size),
            self.screen_size(transform),
        )
    }

    /// True if the given screen point falls inside this item's current
    /// screen rect.
    pub fn contains_screen_point(
        &self,
        point: Point<f64, Physical>,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> bool {
        let r = self.screen_rect(transform, output_size);
        let r = Rectangle::<f64, Physical>::from_loc_and_size(
            (r.loc.x as f64, r.loc.y as f64),
            (r.size.w as f64, r.size.h as f64),
        );
        r.contains(point)
    }

    /// Translate a screen point to surface-local logical coords (texture
    /// natural pixel grid). None if outside the screen rect.
    pub fn local_coords(
        &self,
        screen_point: Point<f64, Physical>,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Option<iced_core::Point> {
        let r = self.screen_rect(transform, output_size);
        let r_f = Rectangle::<f64, Physical>::from_loc_and_size(
            (r.loc.x as f64, r.loc.y as f64),
            (r.size.w as f64, r.size.h as f64),
        );
        if !r_f.contains(screen_point) {
            return None;
        }
        let local = (
            screen_point.x - r.loc.x as f64,
            screen_point.y - r.loc.y as f64,
        );
        // Map the on-screen offset into the surface's iced LOGICAL space — the
        // space iced lays out and hit-tests in (see `IcedRuntime::tick`, which
        // builds the UI at `viewport.logical_size()`). The on-screen rect spans
        // `screen_size` physical px (`size × zoom` for World) while the logical
        // extent is `size / scale_factor`, so the offset scales by
        // `logical / screen = 1 / (zoom × scale_factor)`.
        //
        // For a World surface the zoom counter-scale (`scale_factor = 1/zoom`,
        // set via `request_resize_scaled_by_id`) cancels the projection zoom, so
        // the on-screen size is held constant and the mapping is 1:1 at every
        // zoom. For Screen, zoom is 1 and scale_factor is 1. Dividing by zoom
        // ALONE (the previous code) mis-mapped a zoomed-out World surface — the
        // selection toolbar received clicks at the wrong spot.
        let zoom = match self.space {
            IcedSpace::Screen => 1.0,
            IcedSpace::World => transform.zoom,
        };
        let divisor = zoom * self.inner.scale_factor() as f64;
        Some(iced_core::Point::new(
            (local.0 / divisor) as f32,
            (local.1 / divisor) as f32,
        ))
    }

    // ── Untyped passthrough ───────────────────────────────────────

    pub fn queue_event(&mut self, event: IcedEvent) {
        self.inner.queue_event(event);
    }
    pub fn pointer_leave(&mut self) {
        self.inner.pointer_leave();
    }
    pub fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32) {
        // inner is instance. instance holds runtime
        self.inner.request_resize(new_size, scale_factor);
    }
    pub fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)> {
        self.inner.pending_resize()
    }
    pub fn commit(&self) -> CommitCounter {
        self.inner.commit()
    }

    // ── Type queries ──────────────────────────────────────────────

    pub fn is<U: IcedUi>(&self) -> bool {
        self.type_id == TypeId::of::<IcedInstance<U>>()
    }

    pub fn get<U: IcedUi>(&self) -> Option<&IcedInstance<U>> {
        if self.is::<U>() {
            self.inner.as_any().downcast_ref::<IcedInstance<U>>()
        } else {
            None
        }
    }

    pub fn get_mut<U: IcedUi>(&mut self) -> Option<&mut IcedInstance<U>> {
        if self.is::<U>() {
            self.inner.as_any_mut().downcast_mut::<IcedInstance<U>>()
        } else {
            None
        }
    }

    // ── Internal ──────────────────────────────────────────────────

    pub(crate) fn smithay_id(&self) -> &Id {
        self.inner.smithay_id()
    }
    pub(crate) fn tick(&mut self) -> bool {
        self.inner.tick()
    }
    pub(crate) fn render(&mut self) {
        self.inner.render();
    }
    pub(crate) fn bump_commit(&mut self) {
        self.inner.bump_commit();
    }

    /// Build a render element for this item in screen coordinates.
    pub(crate) fn element_in(
        &self,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> IcedRenderElement {
        let location = self.screen_location(transform, output_size);
        let world_zoom = match self.space {
            IcedSpace::Screen => 1.0,
            IcedSpace::World => transform.zoom,
        };
        IcedRenderElement {
            texture: self.inner.texture_handle().clone(),
            dmabuf: self.inner.dmabuf().clone(),
            space: self.space,
            location,
            size: self.inner.size(),
            world_zoom,
            id: self.inner.smithay_id().clone(),
            commit_counter: self.inner.commit(),
            output: self.output.clone(),
        }
    }

    pub(crate) fn apply_pending_resize(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
    ) -> Result<bool, SurfaceError> {
        self.inner.apply_pending_resize(render_node, wgpu_ctx, gles)
    }
}

impl std::fmt::Debug for IcedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcedItem")
            .field("handle", &self.handle_id())
            .field("space", &self.space)
            .field("location", &self.location())
            .field("size", &self.size())
            .finish()
    }
}

pub(crate) fn build_instance<U: IcedUi>(
    id: HandleId,
    ui: U,
    surface: IcedSurface,
    engine: compositor_support_iced_core_engine_base::SharedEngine,
    location: Point<i32, Physical>,
    scale_factor: f32,
) -> IcedInstance<U> {
    let size_px = (surface.size.w as u32, surface.size.h as u32);
    let runtime = IcedRuntime::new(ui, engine, size_px, scale_factor);
    IcedInstance {
        scale_factor,
        id,
        smithay_id: Id::new(),
        commit: CommitCounter::default(),
        location,
        surface,
        runtime,
        pending_resize: None,
    }
}
