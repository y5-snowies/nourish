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
use std::time::{Duration, Instant};

use iced_core::Event as IcedEvent;

use iced_core::mouse;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::keyboard::ModifiersState;
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_support_iced_core_engine_base::{IcedUi, SharedEngine};
use compositor_monitor_runtime_surface_base::{IcedSurface, WgpuVulkanContext};

use crate::element::IcedRenderElement;
use crate::error::{CreateError, DispatchError, ResizeError};
use crate::handle::{HandleId, IcedHandle};
use crate::instance::{IcedInstance, IcedItem, build_instance};
use crate::space::{IcedSpace, Transform};
use iced_core::keyboard::Modifiers as IcedMods;

/// How long a surface must stay continuously hidden (off-screen or occluded)
/// before its GPU backing is released. Re-allocation is immediate on reveal, so
/// this is deliberately long: it only reclaims memory for surfaces that stay
/// hidden, and never thrashes something skimming the edge or briefly covered.
/// The runtime keeps ticking the whole time.
const BACKING_GRACE: Duration = Duration::from_secs(2);

/// World-surface release debounce (issue: avoid blink/thrash by batching).
/// A surface must be continuously hidden this long before it becomes a release
/// candidate (the per-surface lazy delay).
const HIDDEN_ELIGIBLE: Duration = Duration::from_millis(1200);
/// Trailing debounce: once candidates exist, wait this long with NO new
/// candidate before flushing the whole batch of releases together.
const RELEASE_TRAILING: Duration = Duration::from_millis(500);
/// Hard cap: flush all pending releases at most this long after the FIRST
/// candidate appeared, even if the trailing debounce keeps getting reset.
const RELEASE_MAX: Duration = Duration::from_millis(2000);

/// A surface is considered visible (backing kept) while at least this percent of
/// its area is actually on-screen in some pane after occlusion. Below it — fully
/// off-screen or fully obstructed — the backing is released after the grace.
const MIN_VISIBLE_PERCENT: i64 = 1;

/// One leaf viewport (split / floating pane) being drawn on the current output:
/// the camera it is drawn through and the physical sub-rect it occupies. World
/// iced surfaces are composited once per pane; [`IcedRegistry::manage_backings`]
/// judges visibility from the union of these.
#[derive(Clone, Copy, Debug)]
pub struct PaneView {
    pub transform: Transform,
    pub rect: Rectangle<i32, Physical>,
}

/// Area of `base` left uncovered after subtracting `occluders` (axis-aligned
/// rectangle subtraction). Exact; used for the ≥1%-visible test.
fn visible_area_after(
    base: Rectangle<i32, Physical>,
    occluders: &[Rectangle<i32, Physical>],
) -> i64 {
    let mut frags = vec![base];
    for occ in occluders {
        let mut next = Vec::new();
        for f in frags.drain(..) {
            subtract_rect(f, *occ, &mut next);
        }
        frags = next;
        if frags.is_empty() {
            return 0;
        }
    }
    frags
        .iter()
        .map(|r| r.size.w as i64 * r.size.h as i64)
        .sum()
}

/// Push `f` minus `occ` (0–4 axis-aligned fragments) into `out`.
fn subtract_rect(
    f: Rectangle<i32, Physical>,
    occ: Rectangle<i32, Physical>,
    out: &mut Vec<Rectangle<i32, Physical>>,
) {
    let Some(inter) = f.intersection(occ) else {
        out.push(f);
        return;
    };
    let (fx0, fy0) = (f.loc.x, f.loc.y);
    let (fx1, fy1) = (f.loc.x + f.size.w, f.loc.y + f.size.h);
    let (ix0, iy0) = (inter.loc.x, inter.loc.y);
    let (ix1, iy1) = (inter.loc.x + inter.size.w, inter.loc.y + inter.size.h);
    if iy0 > fy0 {
        out.push(Rectangle::from_loc_and_size((fx0, fy0), (fx1 - fx0, iy0 - fy0)));
    }
    if iy1 < fy1 {
        out.push(Rectangle::from_loc_and_size((fx0, iy1), (fx1 - fx0, fy1 - iy1)));
    }
    if ix0 > fx0 {
        out.push(Rectangle::from_loc_and_size((fx0, iy0), (ix0 - fx0, iy1 - iy0)));
    }
    if ix1 < fx1 {
        out.push(Rectangle::from_loc_and_size((ix1, iy0), (fx1 - ix1, iy1 - iy0)));
    }
}

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

    /// Whether hidden surfaces' GPU backings are released to reclaim memory
    /// (the `release_hidden_surfaces` preference; on by default). When off,
    /// nothing is ever released — surfaces are only ensured/rendered — so the
    /// registry behaves as it did before the de-alloc pass.
    dealloc_enabled: bool,

    /// Compositing z-order for World surfaces, from the external DrawOrder
    /// authority — NOT the `items` Vec order. `draw_pos[id]` is the position in
    /// topmost-first order, so a SMALLER value draws on top. Occlusion uses this:
    /// a World surface raised above another has a lower value here even when it
    /// sits at a lower `items` index.
    draw_pos: HashMap<HandleId, usize>,

    /// Batched-release debounce. `batch_first` = when the first surface became
    /// release-eligible; `batch_touch` = last time a NEW surface became eligible.
    /// Pending releases flush together once the trailing debounce settles
    /// (`RELEASE_TRAILING` since `batch_touch`) or the max window elapses
    /// (`RELEASE_MAX` since `batch_first`) — see `manage_backings`.
    batch_first: Option<Instant>,
    batch_touch: Option<Instant>,
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
            dealloc_enabled: true,
            draw_pos: HashMap::new(),
            batch_first: None,
            batch_touch: None,
        }
    }

    /// Enable/disable releasing hidden surfaces' backings (the
    /// `release_hidden_surfaces` preference). When off, surfaces are only ever
    /// ensured/rendered, never released.
    pub fn set_dealloc_enabled(&mut self, enabled: bool) {
        self.dealloc_enabled = enabled;
    }

    /// Provide the compositor's World draw order (topmost-first) so occlusion is
    /// evaluated against the real z-order rather than the `items` Vec. Ids not
    /// present are treated as not participating in occlusion. Call once per frame.
    pub fn set_draw_order(&mut self, topmost_first: &[HandleId]) {
        self.draw_pos.clear();
        for (pos, id) in topmost_first.iter().enumerate() {
            self.draw_pos.insert(*id, pos);
        }
    }

    /// True if `occluder` is drawn above `base` per the DrawOrder z-map (smaller
    /// position = on top). False if either isn't in the current draw order.
    fn draws_above(&self, occluder: HandleId, base: HandleId) -> bool {
        match (self.draw_pos.get(&occluder), self.draw_pos.get(&base)) {
            (Some(o), Some(b)) => o < b,
            _ => false,
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

    /// Tick every instance (drives animations/messages/async — always), then
    /// rasterize ONLY the surfaces actually visible on the output currently
    /// being drawn. Off-screen surfaces are ticked but not rendered; the GPU
    /// rasterization into their (possibly large) textures is skipped.
    ///
    /// This runs once per output (see the native per-output render loop), each
    /// pass carrying that output's camera in `last_transform`/`last_output_size`
    /// (set by `cache_camera_and_bump` just before). A surface therefore renders
    /// on the first pass whose monitor shows it and is skipped by the others
    /// (its `tick` returns false once rendered) — so it's rasterized iff some
    /// monitor shows it, with only a cheap per-output rect test as overhead.
    /// Tick every runtime (drive animations/messages/async — always), and manage
    /// the backing lifecycle for **Screen-space** surfaces using the single
    /// output camera (`last_transform`), which is correct for them.
    ///
    /// **World-space** surfaces are composited per-pane through per-pane cameras
    /// (the DrawOrder content band), so their visibility can't be judged from one
    /// camera here — [`manage_backings`](Self::manage_backings) handles them from
    /// the union of panes. Here they are only ticked (and acknowledged if dirty so
    /// they don't pin the redraw loop); [`manage_backings`] does their render.
    pub fn process_frame(&mut self, render_node: &str, gles: &mut GlesRenderer) {
        let wgpu = self.wgpu_ctx.clone();

        // Feature OFF: no visibility compute at all — tick and render dirty items
        // exactly as before the de-alloc pass (recovering any surface released
        // while it was on, so a mid-session toggle can't leave a blank surface).
        if !self.dealloc_enabled {
            for item in &mut self.items {
                let due = item.tick();
                if !item.is_resident() {
                    let _ = item.ensure_backing(render_node, &wgpu, gles);
                }
                if due || item.is_stale() {
                    item.render();
                }
            }
            return;
        }

        let transform = self.last_transform;
        let output_size = self.last_output_size;
        // Before the first real camera is cached the viewport is 0×0; treat
        // everything as on-screen then to preserve the pre-optimization path.
        let has_viewport = output_size.w > 0.0 && output_size.h > 0.0;
        let now = Instant::now();

        for item in &mut self.items {
            let due = item.tick();

            if item.space() != IcedSpace::Screen {
                // World: backing + rasterization owned by `manage_backings`.
                // Acknowledge a dirty tick so it doesn't pin the loop; the stale
                // flag makes `manage_backings` re-render it while it's visible.
                if due {
                    item.skip_render();
                }
                continue;
            }

            let on_screen = !has_viewport || item.intersects_viewport(&transform, output_size);
            if on_screen {
                item.mark_on_screen(now);
                if !item.is_resident() {
                    if let Err(e) = item.ensure_backing(render_node, &wgpu, gles) {
                        warn!("iced backing re-alloc failed handle={:?}: {e:?}", item.handle_id());
                        continue;
                    }
                }
                if due || item.is_stale() {
                    item.render();
                }
            } else {
                if due {
                    item.skip_render();
                }
                if item.is_resident()
                    && now.duration_since(item.last_on_screen()) > BACKING_GRACE
                {
                    item.release_backing();
                }
            }
        }
    }

    /// Whether ≥[`MIN_VISIBLE_PERCENT`] of World item `idx`'s on-screen footprint
    /// is actually visible in some pane, after clipping to the pane and
    /// subtracting the opaque occluders drawn ABOVE it (per the DrawOrder z-map,
    /// not the `items` Vec order). Comparing against the on-screen footprint —
    /// not the logical size — keeps a zoomed-out but fully-visible surface counted.
    /// `pane_occ[p]` is the precomputed `(draw_pos, pane-clipped rect)` of every
    /// opaque occluder in pane `p` (see `manage_backings`). An occluder counts
    /// only if it draws ABOVE this item (`draw_pos` strictly smaller).
    fn world_visible(
        &self,
        idx: usize,
        output_size: Size<f64, Physical>,
        panes: &[PaneView],
        pane_occ: &[Vec<(usize, Rectangle<i32, Physical>)>],
    ) -> bool {
        if !self.items[idx].is_visible() {
            return false;
        }
        let base_pos = self.draw_pos.get(&self.items[idx].handle_id()).copied();
        for (p, pane) in panes.iter().enumerate() {
            let rect = self.items[idx].screen_rect(&pane.transform, output_size);
            let footprint = (rect.size.w as i64).max(0) * (rect.size.h as i64).max(0);
            if footprint == 0 {
                continue;
            }
            let Some(clipped) = rect.intersection(pane.rect) else {
                continue;
            };
            let occluders: Vec<Rectangle<i32, Physical>> = match base_pos {
                Some(bp) => pane_occ[p]
                    .iter()
                    .filter(|(dp, _)| *dp < bp)
                    .map(|(_, r)| *r)
                    .collect(),
                None => Vec::new(),
            };
            if visible_area_after(clipped, &occluders) * 100 >= footprint * MIN_VISIBLE_PERCENT {
                return true;
            }
        }
        false
    }

    /// Manage World-space surfaces' backing lifecycle from the union of the
    /// current output's panes. A surface visible in some pane (≥1% after
    /// occlusion) is ensured resident and rendered if stale, immediately on
    /// reveal. A surface continuously hidden past [`HIDDEN_ELIGIBLE`] becomes a
    /// release CANDIDATE; candidates are not freed one-by-one but DEBOUNCED and
    /// flushed in a batch — once the batch settles ([`RELEASE_TRAILING`] with no
    /// new candidate) or the [`RELEASE_MAX`] window elapses — which avoids the
    /// blink/thrash of eager per-frame releases.
    ///
    /// Call once per output in the GLES prepare phase, after `process_frame`.
    pub fn manage_backings(
        &mut self,
        render_node: &str,
        gles: &mut GlesRenderer,
        output_size: Size<f64, Physical>,
        panes: &[PaneView],
    ) {
        if panes.is_empty() || !self.dealloc_enabled {
            return;
        }
        let now = Instant::now();
        let wgpu = self.wgpu_ctx.clone();

        // Precompute per pane the opaque occluders once — their `(draw_pos,
        // pane-clipped rect)` — so each surface's visibility test is O(occluders)
        // instead of re-scanning + re-projecting every item (the old O(n²)).
        let pane_occ: Vec<Vec<(usize, Rectangle<i32, Physical>)>> = panes
            .iter()
            .map(|pane| {
                self.items
                    .iter()
                    .filter(|it| it.is_opaque_occluder() && it.is_visible())
                    .filter_map(|it| {
                        let dp = *self.draw_pos.get(&it.handle_id())?;
                        let r = it
                            .screen_rect(&pane.transform, output_size)
                            .intersection(pane.rect)?;
                        Some((dp, r))
                    })
                    .collect()
            })
            .collect();

        let mut any_candidate = false;
        let mut new_candidate = false;

        for idx in 0..self.items.len() {
            // Screen-space surfaces are handled in `process_frame` (single camera).
            if self.items[idx].space() != IcedSpace::World {
                continue;
            }

            if self.world_visible(idx, output_size, panes, &pane_occ) {
                // Reveal is immediate: ensure + render this frame, cancel pending.
                self.items[idx].mark_on_screen(now);
                self.items[idx].set_release_pending(false);
                if !self.items[idx].is_resident() {
                    if let Err(e) = self.items[idx].ensure_backing(render_node, &wgpu, gles) {
                        warn!(
                            "iced backing re-alloc failed handle={:?}: {e:?}",
                            self.items[idx].handle_id()
                        );
                        continue;
                    }
                }
                if self.items[idx].is_stale() {
                    self.items[idx].render();
                }
            } else if self.items[idx].is_resident()
                && now.duration_since(self.items[idx].last_on_screen()) > HIDDEN_ELIGIBLE
            {
                any_candidate = true;
                if !self.items[idx].release_pending() {
                    self.items[idx].set_release_pending(true);
                    new_candidate = true;
                }
            }
        }

        // Debounce bookkeeping: a new candidate resets the trailing timer and
        // opens the max window; no candidates at all clears the batch.
        if new_candidate {
            self.batch_touch = Some(now);
            if self.batch_first.is_none() {
                self.batch_first = Some(now);
            }
        }
        if !any_candidate {
            self.batch_first = None;
            self.batch_touch = None;
        }

        let settled = self
            .batch_touch
            .is_some_and(|t| now.duration_since(t) >= RELEASE_TRAILING);
        let maxed = self
            .batch_first
            .is_some_and(|t| now.duration_since(t) >= RELEASE_MAX);

        if any_candidate && (settled || maxed) {
            for idx in 0..self.items.len() {
                if self.items[idx].release_pending() {
                    // Still a candidate (visible ones cleared their flag above).
                    self.items[idx].release_backing();
                }
            }
            self.batch_first = None;
            self.batch_touch = None;
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
                if !in_layer || !i.is_visible() || !i.is_resident() {
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
        self.process_frame(render_node, gles);
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
        self.process_frame(render_node, gles);
        Ok(())
    }

    /// Render a SINGLE surface by id in a specific pane. Lets the driver
    /// interleave iced surfaces with other drawables by the world DrawOrder
    /// instead of the monolithic, layer-batched `render_all`.
    ///
    /// Occlusion is evaluated **per pane** here (an opaque occluder above that
    /// fully covers this surface in this pane) — distinct from the union
    /// visibility `manage_backings` uses for the backing lifecycle. Returns
    /// `None` for a hidden, released, or fully-covered surface.
    pub fn element_of(
        &self,
        id: HandleId,
        transform: &Transform,
        output_size: Size<f64, Physical>,
    ) -> Option<IcedRenderElement> {
        let &idx = self.index.get(&id)?;
        let item = self.items.get(idx)?;
        if !item.is_visible() || !item.is_resident() {
            return None;
        }
        // Per-pane occlusion (gated by the de-alloc feature — its whole purpose is
        // to pair with backing release): skip drawing a surface fully covered by
        // an opaque occluder drawn ABOVE it (per the DrawOrder z-map) in this pane.
        let rect = item.screen_rect(transform, output_size);
        let covered = self.dealloc_enabled
            && self.items.iter().any(|it| {
                it.is_opaque_occluder()
                    && it.is_visible()
                    && self.draws_above(it.handle_id(), id)
                    && it.screen_rect(transform, output_size).contains_rect(rect)
            });
        if covered {
            return None;
        }
        Some(item.element_in(transform, output_size))
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

    /// Mark a surface as an opaque occluder: it fully covers its rect, so any
    /// surface entirely behind it (in draw order) is skipped — not rasterized
    /// and not composited — while covered. Use for backgrounds that paint their
    /// whole extent opaquely (e.g. launcher placeholders). Off by default.
    pub fn set_opaque_occluder_by_id(&mut self, id: HandleId, opaque: bool) -> bool {
        match self.get_mut(id) {
            Some(item) => {
                item.set_opaque_occluder(opaque);
                true
            }
            None => false,
        }
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
