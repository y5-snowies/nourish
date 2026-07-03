//! `IcedRegistry`: the compositor-facing API for all Iced instances.
//!
//! ## Storage
//! `Vec<IcedItem>` in draw order (back to front), `HashMap<HandleId, usize>`
//! for O(1) lookup.
//!
//! ## Camera/transform is NOT stored
//! Mirroring how `window.render_elements(...)` accepts screen position
//! and zoom as arguments, this registry takes a `Transform` and output
//! size as parameters to render and hit-test calls. The compositor owns
//! the camera; the registry just applies what it's given.
//!
//! Internally the registry caches the last-passed transform so it can
//! bump commit counters on World items when the camera changes (smithay's
//! damage tracker needs that to damage old + new screen rects).
//!
//! ## Per-frame
//! ```ignore
//! let elements = registry.render_all(gles, &transform, output_size)?;
//! ```
//! - Applies pending resizes
//! - Ticks dirty items
//! - Renders re-rendered items into their textures
//! - Returns elements ready to draw at screen coords
//!
//! ## Per-input
//! ```ignore
//! if let Some(handle) = registry.dispatch_pointer_at(point, &transform, output_size) {
//!     // swallow — pointer is over an iced item
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use iced_core::Event as IcedEvent;

use iced_core::mouse;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::keyboard::ModifiersState;
use smithay::utils::{Physical, Point, Size};
use compositor_support_iced_core_engine_base::{IcedUi, SharedEngine};
use compositor_monitor_runtime_surface_base::{IcedSurface, WgpuVulkanContext};

use crate::element::IcedRenderElement;
use crate::error::{CreateError, DispatchError, ResizeError};
use crate::handle::{HandleId, IcedHandle};
use crate::instance::{IcedInstance, IcedItem, build_instance};
use crate::space::{IcedSpace, Transform};
use iced_core::keyboard::Modifiers as IcedMods;

pub struct IcedRegistry {
    engine: SharedEngine,
    wgpu_ctx: Arc<WgpuVulkanContext>,

    items: Vec<IcedItem>,
    index: HashMap<HandleId, usize>,

    next_id: u64,
    pointer_inside: Option<HandleId>,
    keyboard_focus: Option<HandleId>,
    pointer_grab: Option<HandleId>,

    /// Effective modifiers under incremental semantics: only modifiers
    /// actually pressed during the current focus count as held. Resets
    /// to empty on every focus change.
    effective_modifiers: IcedMods,
    /// Cached so we can detect camera changes and bump World items' commits
    /// for damage tracking. Not authoritative — the compositor owns the
    /// real camera and passes it each render call.
    last_transform: Transform,
    last_output_size: Size<f64, Physical>,

    instance_scale: f32,
}

impl std::fmt::Debug for IcedRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcedRegistry")
            .field("instance_count", &self.items.len())
            .field("pointer_inside", &self.pointer_inside)
            .finish()
    }
}

impl IcedRegistry {
    pub fn new(engine: SharedEngine, wgpu_ctx: Arc<WgpuVulkanContext>) -> Self {
        Self {
            effective_modifiers: IcedMods::empty(),
            engine,
            wgpu_ctx,
            items: Vec::new(),
            index: HashMap::new(),
            next_id: 1,
            pointer_inside: None,
            keyboard_focus: None,
            pointer_grab: None,
            last_transform: Transform::identity(),
            last_output_size: Size::from((0.0, 0.0)),
            instance_scale: 1.0,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn contains(&self, id: HandleId) -> bool {
        self.index.contains_key(&id)
    }
    pub fn set_instance_scale(&mut self, scale: f32) {
        self.instance_scale = scale;
        for item in &mut self.items {
            item.request_resize(item.size(), scale);
        }
    }

    // ── Iteration / lookup ────────────────────────────────────────

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &IcedItem> + ExactSizeIterator + '_ {
        self.items.iter()
    }

    pub fn iter_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut IcedItem> + ExactSizeIterator + '_ {
        self.items.iter_mut()
    }
    pub fn get(&self, id: HandleId) -> Option<&IcedItem> {
        self.index.get(&id).and_then(|&idx| self.items.get(idx))
    }
    pub fn get_mut(&mut self, id: HandleId) -> Option<&mut IcedItem> {
        let idx = *self.index.get(&id)?;
        self.items.get_mut(idx)
    }

    // ── Lifecycle ─────────────────────────────────────────────────

    pub fn create<U: IcedUi>(
        &mut self,
        render_node: &str,
        ui: U,
        gles: &mut GlesRenderer,
        location: Point<i32, Physical>,
        size: Size<i32, Physical>,
        layer: u64,
    ) -> Result<IcedHandle<U>, CreateError> {
        self.create_in_space(
            render_node,
            ui,
            gles,
            location,
            size,
            IcedSpace::World,
            layer,
        )
    }

    pub fn create_screen<U: IcedUi>(
        &mut self,
        render_node: &str,
        ui: U,
        gles: &mut GlesRenderer,
        location: Point<i32, Physical>,
        size: Size<i32, Physical>,
        layer: u64,
    ) -> Result<IcedHandle<U>, CreateError> {
        self.create_in_space(
            render_node,
            ui,
            gles,
            location,
            size,
            IcedSpace::Screen,
            layer,
        )
    }

    pub fn create_in_space<U: IcedUi>(
        &mut self,
        render_node: &str,
        ui: U,
        gles: &mut GlesRenderer,
        location: Point<i32, Physical>,
        size: Size<i32, Physical>,
        space: IcedSpace,
        layer: u64,
    ) -> Result<IcedHandle<U>, CreateError> {
        let id = HandleId(self.next_id);
        self.next_id += 1;

        let surface = IcedSurface::allocate(render_node, &self.wgpu_ctx, gles, size)?;
        let instance = build_instance(
            id,
            ui,
            surface,
            self.engine.clone(),
            location,
            self.instance_scale,
        );

        trace!("created iced instance handle={id:?} location={location:?} size={size:?} space={space:?}");

        let item = IcedItem::new(instance, space, layer);
        let idx = self.items.len();
        self.items.push(item);
        self.index.insert(id, idx);
        Ok(IcedHandle::new(id))
    }

    pub fn destroy<U: IcedUi>(&mut self, handle: IcedHandle<U>) -> bool {
        self.destroy_by_id(handle.id)
    }

    pub fn destroy_by_id(&mut self, id: HandleId) -> bool {
        let Some(idx) = self.index.remove(&id) else {
            return false;
        };
        self.items.remove(idx);
        for (_, i) in self.index.iter_mut() {
            if *i > idx {
                *i -= 1;
            }
        }
        if self.pointer_inside == Some(id) {
            self.pointer_inside = None;
        }

        if self.pointer_inside == Some(id) {
            self.pointer_inside = None;
        }
        if self.keyboard_focus == Some(id) {
            self.keyboard_focus = None;
            self.effective_modifiers = IcedMods::empty();
        }
        if self.pointer_grab == Some(id) {
            self.pointer_grab = None;
        }

        trace!("destroyed iced instance handle={id:?}");
        true
    }

    // ── Placement & z-order ────────────────────────────────────────

    pub fn set_location<U: IcedUi>(
        &mut self,
        handle: IcedHandle<U>,
        location: Point<i32, Physical>,
    ) -> bool {
        self.set_location_by_id(handle.id, location)
    }

    pub fn set_location_by_id(&mut self, id: HandleId, location: Point<i32, Physical>) -> bool {
        match self.get_mut(id) {
            Some(item) => {
                item.set_location(location);
                true
            }
            None => false,
        }
    }

    pub fn raise(&mut self, id: HandleId) {
        let Some(&idx) = self.index.get(&id) else {
            return;
        };
        if idx == self.items.len() - 1 {
            return;
        }
        let item = self.items.remove(idx);
        self.items.push(item);
        for (other_id, i) in self.index.iter_mut() {
            if *other_id == id {
                *i = self.items.len() - 1;
            } else if *i > idx {
                *i -= 1;
            }
        }
    }

    pub fn lower(&mut self, id: HandleId) {
        let Some(&idx) = self.index.get(&id) else {
            return;
        };
        if idx == 0 {
            return;
        }
        let item = self.items.remove(idx);
        self.items.insert(0, item);
        for (other_id, i) in self.index.iter_mut() {
            if *other_id == id {
                *i = 0;
            } else if *i < idx {
                *i += 1;
            }
        }
    }

    pub fn location_of(&self, id: HandleId) -> Option<Point<i32, Physical>> {
        self.get(id).map(|i| i.location())
    }
    pub fn size_of(&self, id: HandleId) -> Option<Size<i32, Physical>> {
        self.get(id).map(|i| i.size())
    }
    pub fn space_of(&self, id: HandleId) -> Option<IcedSpace> {
        self.get(id).map(|i| i.space())
    }

    // ── Resize (deferred) ──────────────────────────────────────────

    pub fn request_resize<U: IcedUi>(
        &mut self,
        handle: IcedHandle<U>,
        new_size: Size<i32, Physical>,
    ) -> bool {
        self.request_resize_by_id(handle.id, new_size)
    }

    pub fn request_resize_by_id(&mut self, id: HandleId, new_size: Size<i32, Physical>) -> bool {
        let instance_scale = self.instance_scale;
        match self.get_mut(id) {
            Some(item) => {
                item.request_resize(new_size, instance_scale);
                true
            }
            None => false,
        }
    }

    /// Resize with an explicit per-instance iced scale factor. Used by a
    /// world-space surface that counter-scales with zoom: `new_size = base/zoom`
    /// keeps the on-screen size constant, and `scale_factor = 1/zoom` keeps the
    /// content laid out at the native `base` logical size (so it fills).
    pub fn request_resize_scaled_by_id(
        &mut self,
        id: HandleId,
        new_size: Size<i32, Physical>,
        scale_factor: f32,
    ) -> bool {
        match self.get_mut(id) {
            Some(item) => {
                item.request_resize(new_size, scale_factor);
                true
            }
            None => false,
        }
    }

    pub fn apply_pending_resizes(
        &mut self,
        render_node: &str,
        gles: &mut GlesRenderer,
    ) -> Result<usize, ResizeError> {
        let mut applied = 0;
        for item in &mut self.items {
            match item.apply_pending_resize(render_node, &self.wgpu_ctx, gles) {
                Ok(true) => applied += 1,
                Ok(false) => {}
                Err(e) => warn!("resize failed handle={:?} error={e:?}", item.handle_id()),
            }
        }
        Ok(applied)
    }

    // ── Event dispatch ─────────────────────────────────────────────
    pub fn dispatch_event(&mut self, id: HandleId, event: IcedEvent) -> Result<(), DispatchError> {
        let item = self.get_mut(id).ok_or(DispatchError::UnknownHandle(id))?;
        item.queue_event(event);
        Ok(())
    }

    pub fn dispatch_message<U: IcedUi>(
        &mut self,
        handle: IcedHandle<U>,
        message: U::Message,
    ) -> Result<(), DispatchError> {
        let item = self
            .get_mut(handle.id)
            .ok_or(DispatchError::UnknownHandle(handle.id))?;
        let typed = item.get_mut::<U>().ok_or(DispatchError::TypeMismatch)?;
        typed.runtime_mut().queue_message(message);
        Ok(())
    }

    pub fn instance_mut<U: IcedUi>(
        &mut self,
        handle: IcedHandle<U>,
    ) -> Option<&mut IcedInstance<U>> {
        self.get_mut(handle.id).and_then(|i| i.get_mut::<U>())
    }
    pub fn instance<U: IcedUi>(&self, handle: IcedHandle<U>) -> Option<&IcedInstance<U>> {
        self.get(handle.id).and_then(|i| i.get::<U>())
    }

    /// Route a pointer position to a specific (already-hit-tested) iced item.
    /// Drives Enter/Leave bookkeeping based on the previous target.
    /// `target = None` means "pointer is not over any iced item" (i.e., it's
    /// over a window or empty space).
    pub fn route_pointer_to(
        &mut self,
        target: Option<HandleId>,
        screen_point: Point<f64, Physical>,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) {
        // Enter/Leave transitions.
        if self.pointer_inside != target {
            if let Some(prev) = self.pointer_inside {
                if let Some(item) = self.get_mut(prev) {
                    item.pointer_leave();
                }
            }
            if let Some(now) = target {
                let t = *transform;
                if let Some(item) = self.get_mut(now) {
                    if let Some(local) = item.local_coords(screen_point, &t, output_size) {
                        item.queue_event(IcedEvent::Mouse(mouse::Event::CursorEntered));
                        item.queue_event(IcedEvent::Mouse(mouse::Event::CursorMoved {
                            position: local,
                        }));
                    }
                }
            }
            self.pointer_inside = target;
        } else if let Some(now) = target {
            // Same target, just motion.
            let t = *transform;
            if let Some(item) = self.get_mut(now) {
                if let Some(local) = item.local_coords(screen_point, &t, output_size) {
                    item.queue_event(IcedEvent::Mouse(mouse::Event::CursorMoved {
                        position: local,
                    }));
                }
            }
        }
    }

    // ── Pointer routing ────────────────────────────────────────────

    /// Move the pointer to a screen point. The transform + output size
    /// are needed to hit-test World items correctly.
    pub fn dispatch_pointer_at(
        &mut self,
        point: Point<f64, Physical>,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Option<HandleId> {
        let hit = self.hit_test(point, transform, output_size);
        self.route_pointer_to(hit, point, transform, output_size);
        hit
    }

    pub fn pointer_global_leave(&mut self) {
        if let Some(prev) = self.pointer_inside.take() {
            if let Some(item) = self.get_mut(prev) {
                item.pointer_leave();
            }
        }
    }

    pub fn pointer_target(&self) -> Option<HandleId> {
        self.pointer_inside
    }

    // ── Keyboard focus ────────────────────────────────────────────

    /// Set keyboard focus. The previous focus (if any) receives a
    /// `window::Event::Unfocused`; the new focus (if any and different)
    /// receives a `window::Event::Focused`.
    ///
    /// Passing `None` clears focus.
    pub fn set_keyboard_focus(&mut self, target: Option<HandleId>) {
        if self.keyboard_focus == target {
            return;
        }

        let mods_empty = self.effective_modifiers.is_empty();

        if let Some(prev) = self.keyboard_focus {
            if let Some(item) = self.get_mut(prev) {
                // Release modifiers from the old focus before unfocusing it.
                if !mods_empty {
                    item.queue_event(IcedEvent::Keyboard(
                        iced_core::keyboard::Event::ModifiersChanged(IcedMods::empty()),
                    ));
                }
                item.queue_event(IcedEvent::Window(iced_core::window::Event::Unfocused));
            }
        }
        if let Some(now) = target {
            if let Some(item) = self.get_mut(now) {
                item.queue_event(IcedEvent::Window(iced_core::window::Event::Focused));
            }
        }

        self.keyboard_focus = target;
        // Incremental: new focus starts empty regardless of held OS modifiers.
        self.effective_modifiers = IcedMods::empty();
    }

    pub fn keyboard_focus(&self) -> Option<HandleId> {
        self.keyboard_focus
    }

    pub fn release_all_modifiers(&mut self) {
        if !self.effective_modifiers.is_empty() {
            self.effective_modifiers = IcedMods::empty();
            if let Some(focused) = self.keyboard_focus {
                let _ = self.dispatch_event(
                    focused,
                    IcedEvent::Keyboard(iced_core::keyboard::Event::ModifiersChanged(
                        IcedMods::empty(),
                    )),
                );
            }
        }
    }

    /// Tell the registry a modifier key was pressed or released. Maintains
    /// effective modifier state under incremental semantics. Dispatches
    /// `ModifiersChanged` to the focused item when the effective state
    /// actually changes.
    pub fn modifier_changed(&mut self, modifier: IcedMods, pressed: bool) {
        let new_state = if pressed {
            self.effective_modifiers | modifier
        } else {
            self.effective_modifiers & !modifier
        };
        if new_state != self.effective_modifiers {
            self.effective_modifiers = new_state;
            if let Some(focused) = self.keyboard_focus {
                let _ = self.dispatch_event(
                    focused,
                    IcedEvent::Keyboard(iced_core::keyboard::Event::ModifiersChanged(new_state)),
                );
            }
        }
    }

    pub fn effective_modifiers(&self) -> IcedMods {
        self.effective_modifiers
    }

    /// Dispatch a pointer-axis (scroll) event to an iced target.
    ///
    /// `target = None` is a no-op. `discrete_x/y` are tick counts (mouse
    /// wheel); `pixel_x/y` are continuous deltas (touchpad). Iced prefers
    /// discrete when present; the translation helper picks the right form.
    pub fn dispatch_axis(
        &mut self,
        target: Option<HandleId>,
        discrete_x: i32,
        discrete_y: i32,
        pixel_x: f64,
        pixel_y: f64,
    ) {
        let Some(handle) = target else { return };
        if let Some(e) = crate::input::wheel_scrolled(discrete_x, discrete_y, pixel_x, pixel_y) {
            let _ = self.dispatch_event(handle, e);
        }
    }

    // ── Button dispatch with grab ─────────────────────────────────

    /// Dispatch a pointer button event to an iced target.
    ///
    /// - On **press** (`pressed = true`): event goes to `target`. If
    ///   `target` is `Some`, that handle is recorded as the pointer grab
    ///   holder. If `target` is `None`, nothing happens.
    /// - On **release** (`pressed = false`): if a grab is active, the
    ///   event goes to the grab holder regardless of `target`. The grab
    ///   is then cleared. This mirrors Wayland's pointer-grab semantics:
    ///   a click that started inside a widget completes on that widget
    ///   even if the cursor moved off before release.
    pub fn dispatch_button(&mut self, target: Option<HandleId>, button_code: u32, pressed: bool) {
        let resolved_target = if pressed {
            target
        } else {
            // Release: prefer the grab holder.
            self.pointer_grab.or(target)
        };

        if let Some(handle) = resolved_target {
            let event = if pressed {
                crate::input::button_pressed(button_code)
            } else {
                crate::input::button_released(button_code)
            };
            if let Some(e) = event {
                let _ = self.dispatch_event(handle, e);
            }
        }

        // Update grab state.
        if pressed {
            self.pointer_grab = target;
        } else {
            self.pointer_grab = None;
        }
    }

    pub fn pointer_grab(&self) -> Option<HandleId> {
        self.pointer_grab
    }

    // ── Hit testing ────────────────────────────────────────────────

    /// Topmost item under the given screen point, considering the given
    /// camera transform for World items.
    pub fn hit_test(
        &self,
        point: Point<f64, Physical>,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Option<HandleId> {
        for item in self.items.iter().rev() {
            // Passthrough (tooltips) and hidden items never intercept input.
            if item.is_passthrough() || !item.is_visible() {
                continue;
            }
            if item.contains_screen_point(point, transform, output_size) {
                return Some(item.handle_id());
            }
        }
        None
    }

    // ── Per-frame ──────────────────────────────────────────────────

    /// Compute screen-space render elements with the given camera.
    /// Bumps World-item commits if the camera changed since the last call.
    fn cache_camera_and_bump(&mut self, transform: Transform, output_size: Size<f64, Physical>) {
        let camera_changed =
            self.last_transform != transform || self.last_output_size != output_size;
        self.last_transform = transform;
        self.last_output_size = output_size;
        if camera_changed {
            for item in &mut self.items {
                if item.space() == IcedSpace::World {
                    item.bump_commit();
                }
            }
        }
    }

    pub fn process_frame(&mut self) {
        for item in &mut self.items {
            if item.tick() {
                item.render();
            }
        }
    }

    /// Build a render element for every item, in draw order, applying
    /// the given camera to World items.
    pub fn elements(
        &self,
        transform: &Transform,
        output_size: Size<f64, Physical>,
        layer: u64,
    ) -> Vec<IcedRenderElement> {
        // If rev() is actually fine here. should probably fix in scene
        self.items
            .iter()
            .rev()
            .filter_map(|i| {
                let in_layer = (i.layer & layer) != 0;
                if !in_layer || !i.is_visible() {
                    return None;
                }

                Some(i.element_in(transform, output_size))
            })
            .collect()
    }

    /// Convenience: apply pending resizes, tick + render dirty items,
    /// then produce render elements with the given camera.
    // CHECK: Move render_node to String reference - so its not made every render
    pub fn render_all(
        &mut self,
        render_node: &str,
        gles: &mut GlesRenderer,
        transform: Transform,
        output_size: Size<f64, Physical>,
        layer: u64,
    ) -> Result<Vec<IcedRenderElement>, ResizeError> {
        self.apply_pending_resizes(render_node, gles)?;
        self.cache_camera_and_bump(transform, output_size);
        self.process_frame();
        Ok(self.elements(&transform, output_size, layer))
    }

    /// Per-frame preparation shared by `render_all` and per-id rendering: apply
    /// pending resizes, cache the camera, advance the iced runtime once. Call
    /// ONCE per frame before any `element_of` calls.
    pub fn prepare_frame(
        &mut self,
        render_node: &str,
        gles: &mut GlesRenderer,
        transform: Transform,
        output_size: Size<f64, Physical>,
    ) -> Result<(), ResizeError> {
        self.apply_pending_resizes(render_node, gles)?;
        self.cache_camera_and_bump(transform, output_size);
        self.process_frame();
        Ok(())
    }

    /// Render a SINGLE surface by id (after `prepare_frame`). Lets the driver
    /// interleave iced surfaces with other drawables by the world DrawOrder
    /// instead of the monolithic, layer-batched `render_all`.
    pub fn element_of(
        &self,
        id: HandleId,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Option<IcedRenderElement> {
        self.get(id)
            .filter(|item| item.is_visible())
            .map(|item| item.element_in(transform, output_size))
    }

    // ── Visibility & passthrough ──────────────────────────────────

    /// Whether the item currently renders and participates in hit-testing.
    pub fn is_visible(&self, id: HandleId) -> bool {
        self.get(id).map(|i| i.is_visible()).unwrap_or(false)
    }

    /// Show or hide an item without destroying it. Hidden items are skipped by
    /// `elements`/`element_of` and never hit-tested. The texture is retained,
    /// so showing again is free (no reallocation).
    pub fn set_visible_by_id(&mut self, id: HandleId, visible: bool) -> bool {
        match self.get_mut(id) {
            Some(item) => {
                item.set_visible(visible);
                true
            }
            None => false,
        }
    }

    /// Bind a surface to a physical output (or unbind with `None`). A bound
    /// screen surface is drawn — and hit-tested — only on that output, so an
    /// overlay can be replicated once per monitor (e.g. the capture stop button)
    /// without the others picking it up. See [`IcedItem::output`].
    pub fn set_output_affinity_by_id(&mut self, id: HandleId, output: Option<String>) -> bool {
        match self.get_mut(id) {
            Some(item) => {
                item.set_output(output);
                true
            }
            None => false,
        }
    }

    /// The output a surface is bound to, if any.
    pub fn output_affinity(&self, id: HandleId) -> Option<String> {
        self.get(id).and_then(|i| i.output().map(str::to_owned))
    }

    pub fn is_passthrough(&self, id: HandleId) -> bool {
        self.get(id).map(|i| i.is_passthrough()).unwrap_or(false)
    }

    /// Mark an item click-through: excluded from `hit_test` so it never
    /// captures the pointer or steals events from what's behind it.
    pub fn set_passthrough_by_id(&mut self, id: HandleId, passthrough: bool) -> bool {
        match self.get_mut(id) {
            Some(item) => {
                item.set_passthrough(passthrough);
                true
            }
            None => false,
        }
    }

    // ── Tooltip surfaces ──────────────────────────────────────────

    /// Create a tooltip-style surface: screen-space, click-through, and
    /// initially hidden.
    ///
    /// A tooltip is a full iced instance like any other — you supply its
    /// `U: IcedUi` (e.g. a small text view) — but it floats above everything at
    /// an arbitrary screen position, independent of the surface it annotates,
    /// and never intercepts pointer input. This is how a tip shows *outside*
    /// the registered texture of the thing being hovered (e.g. a selection
    /// menu): it is simply a separate surface positioned freely.
    ///
    /// Drive it with [`show_tooltip_by_id`](Self::show_tooltip_by_id) /
    /// [`hide_tooltip_by_id`](Self::hide_tooltip_by_id). Update its content with
    /// `dispatch_message` (or by mutating its `ui`), exactly as for any
    /// instance. The `layer` bitmask must be one your per-frame `render_all`
    /// call includes, placed in a band drawn above the annotated surface.
    pub fn create_tooltip<U: IcedUi>(
        &mut self,
        render_node: &str,
        ui: U,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
        layer: u64,
    ) -> Result<IcedHandle<U>, CreateError> {
        let handle = self.create_screen(render_node, ui, gles, Point::from((0, 0)), size, layer)?;
        if let Some(item) = self.get_mut(handle.id) {
            item.set_passthrough(true);
            item.set_visible(false);
        }
        Ok(handle)
    }

    /// Position a tooltip at a screen point, make it visible, and raise it to
    /// the top of the draw order.
    pub fn show_tooltip_by_id(&mut self, id: HandleId, at: Point<i32, Physical>) -> bool {
        if !self.contains(id) {
            return false;
        }
        self.set_location_by_id(id, at);
        self.set_visible_by_id(id, true);
        self.raise(id);
        true
    }

    /// Hide a tooltip (does not destroy it; show it again later).
    pub fn hide_tooltip_by_id(&mut self, id: HandleId) -> bool {
        self.set_visible_by_id(id, false)
    }

    // ── Frame scheduling ──────────────────────────────────────────

    /// True if any instance still wants to be rendered next frame — dirty or
    /// mid-animation. The host's frame loop should schedule another redraw
    /// while this is true so time-based animations keep advancing.
    pub fn wants_frame(&self) -> bool {
        self.items.iter().any(|i| i.wants_frame())
    }
}
// Add this to your compositor binary's keyboard handler module
// (NOT in compositor_monitor_compositor_iced_base crate — keep that one smithay-agnostic).

use crate::input::{KeyboardModifiers, keyboard_event};
use smithay::backend::input::KeyState as SmithayKeyState;
use smithay::input::keyboard::ModifiersState as SmithayModifiersState;

/// Translate a smithay keyboard event into an iced event.
///
/// Returns `None` if the keysym doesn't map to anything iced understands
/// (rare — mostly raw xkb codes for compose sequences and similar).
pub fn translate_keyboard(
    keysym_raw: u32,
    utf8: Option<&str>,
    key_state: SmithayKeyState,
    modifiers: IcedMods,
    is_repeat: bool,
) -> Option<IcedEvent> {
    let pressed = matches!(key_state, SmithayKeyState::Pressed);
    keyboard_event(keysym_raw, utf8, pressed, is_repeat, modifiers)
}
//     let mods = Modifiers {
//         shift: modifiers.shift,
//         ctrl: modifiers.ctrl,
//         alt: modifiers.alt,
//         logo: modifiers.logo,
//     };
//
//     if let Some(e) = iced_input::keyboard_event(
//         keysym_raw,
//         utf8.as_deref(),
//         pressed,
//         false, // is_repeat — not exposed in this callback
//         mods,
//     ) {
// }
