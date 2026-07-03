use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::input::keyboard::LedState;
use smithay::input::dnd::DndGrabHandler;
use smithay::input::pointer::CursorImageStatus;
use smithay::reexports::calloop::{self, LoopHandle};
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::Weak;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::wayland::dmabuf::{DmabufGlobal, ImportNotifier};
use smithay::wayland::drm_syncobj::DrmSyncPointSource;
use smithay::wayland::pointer_constraints::with_pointer_constraint;
use smithay::wayland::shell::wlr_layer::{Layer as WlrLayer, LayerSurface};
use smithay::wayland::shell::xdg::ToplevelSurface;
use std::collections::HashMap;
use smithay::reexports::wayland_server::{Dispatch as SmithayDispatch, GlobalDispatch};
use smithay::wayland::Dispatch2;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::CompositorHandler;
use smithay::wayland::dmabuf::DmabufHandler;
use smithay::wayland::fractional_scale::FractionalScaleHandler;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{DataDeviceHandler, WaylandDndGrabHandler};
use smithay::wayland::shell::wlr_layer::WlrLayerShellHandler;
use smithay::wayland::shell::xdg::XdgShellHandler;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shm::ShmHandler;
use smithay::wayland::xdg_activation::XdgActivationHandler;
use smithay::wayland::xdg_foreign::XdgForeignState;

// 1. Define the trait with your desired traits as bounds (supertraits)
pub trait DispatchWire:
    SeatHandler<PointerFocus = WlSurface>
    + XdgShellHandler
    + SelectionHandler
    + DataDeviceHandler
    + CompositorHandler
    + DmabufHandler
    + XdgDecorationHandler
    + OutputHandler
    + WlrLayerShellHandler
    + DndGrabHandler
    + FractionalScaleHandler
    + WaylandDndGrabHandler
    + XdgActivationHandler
    + ShmHandler
    + BufferHandler
    + 'static
{
}

// 2. Provide a blanket implementation for any type 'T' that satisfies the bounds
// impl<T: XdgShellHandler + SeatHandler> HandleBASE for T {}
// pub trait DispatchWire: XdgShellHandler + SeatHandler {}
// SeatHandler<KeyboardFocus = WlSurface, PointerFocus = WlSurface, TouchFocus = WlSurface>

// ── Dispatch: the concrete, non-generic wayland protocol state ─────────────────
// The wayland dispatch type `D` IS this struct now (document/SMITHAY_DECOUPLING.md
// → "P2 flip"). It defers no trait bound to a generic handler type; the seat is
// `Seat<Dispatch>`, so `impl SeatHandler for Dispatch` MUST live here (it is
// required at the struct definition). ALL the other smithay handler impls also
// live in this crate (orphan rule — see the `handler_impls` module below).
pub struct Dispatch {
    pub dmabuf: compositor_support_smithay_state_dmabuf_base::state::DMABufState,
    pub clipboard: compositor_support_smithay_state_clipboard_base::state::Clipboard,
    pub seat: compositor_support_smithay_state_seat_base::state::Seat<Dispatch>,
    pub xdg_shell: compositor_support_smithay_state_xdg_shell_base::state::XDGShell,
    pub xdg_activation: compositor_support_smithay_state_xdg_activation_base::state::Activation,
    pub xdg_decoration: compositor_support_smithay_state_xdg_decoration_base::state::Decoration,
    pub xdg_foreign_state: compositor_support_smithay_state_xdg_foreign_base::state::Foreign,
    pub shm: compositor_support_smithay_state_shm_base::state::SHMState,
    pub output: compositor_support_smithay_state_output_base::state::OutputState,
    pub popup: compositor_support_smithay_state_popup_base::state::PopupState,
    pub layershell: compositor_support_smithay_state_layershell_base::state::Layershell,
    // `space` moved out of Dispatch: the window Space is now owned by the
    // spatial world (document/ARCHITECTURE.md → "Window tracking"). Smithay
    // handlers reach it via `WireTrait::host_space[_mut]`.
    pub compositor: compositor_support_smithay_state_compositor_base::state::Compositor,
    pub presentation: compositor_support_smithay_state_presentation_base::state::Presentation,
    pub viewporter: compositor_support_smithay_state_viewporter_base::state::Viewporter,
    pub cursor_shape: compositor_support_smithay_state_cursor_shape_base::state::CursorShape,
    pub fractional: compositor_support_smithay_state_fractional_base::state::Fractional,
    pub text_input: compositor_support_smithay_state_text_input_base::state::TextInput,
    pub dnd: compositor_support_smithay_state_dnd_base::state::DNDState,
    pub singlepixel: compositor_support_smithay_state_singlepixel_base::state::SinglePixel,
    pub needs_redraw: bool,

    // Additional safety for ping
    pub render_in_flight: bool,

    pub redraw_ping: Option<calloop::ping::Ping>,

    // Protocol outboxes — handlers record here (world-free); the rim drains them
    // after dispatch_clients + applies world effects. document/SMITHAY_DECOUPLING.md
    pub committed: Vec<WlSurface>,
    pub new_toplevels: Vec<Window>,
    pub destroyed_toplevels: Vec<ToplevelSurface>,
    pub fullscreen_requests: Vec<(ToplevelSurface, bool)>,
    pub new_layers: Vec<(LayerSurface, Option<WlOutput>, WlrLayer, String)>,
    pub destroyed_layers: Vec<LayerSurface>,
    pub pending_dmabuf: Vec<(DmabufGlobal, Dmabuf, ImportNotifier)>,
    pub geometries: HashMap<WlSurface, Rectangle<i32, Logical>>,
    // Pointer-constraint restoration tokens (seat warp); drain performs them.
    pub pending_restoration: Vec<(WlSurface, Point<f64, Logical>)>,
    // Syncobj fence sources recorded by the pre-commit hook (which has no
    // loop_handle); the rim drain inserts them via `Wire::loop_handle`.
    pub pending_blockers: Vec<(Weak<WlSurface>, DrmSyncPointSource)>,
    // Deferred `set_data_device_focus` (needs DataDeviceHandler — downstream);
    // recorded by `focus_changed`, applied by the rim drain. The inner
    // `Option<Client>` is the focused client (None == clear focus).
    pub pending_data_focus: Option<Option<Client>>,
}

// ── Redraw scheduling (inlined; handlers call these on Dispatch) ──────────────
impl Dispatch {
    #[inline]
    pub fn schedule_redraw_post_vblank(&mut self) { self.needs_redraw = true; }
    #[inline]
    pub fn schedule_redraw(&mut self) {
        if self.needs_redraw { return; }
        self.needs_redraw = true;
        if !self.render_in_flight {
            if let Some(p) = &self.redraw_ping { p.ping(); }
        }
    }
    /// Like `schedule_redraw` but ALWAYS fires the redraw ping, even while a frame is
    /// in flight on another pipe. Needed when a NEW output pipe comes up (hotplug /
    /// reactivation): it has never flipped, so it gets no per-CRTC vblank and the
    /// vblank path (`RenderScope::Crtc`) never renders it — only an `All` render (the
    /// ping) gives it its first frame and starts its own vblank cycle. The ping's
    /// `execute(All)` skips per-pipe `in_flight` outputs, so forcing it mid-flight only
    /// renders the idle (new) pipe. Without this the new output stays dark until a full
    /// resume render (e.g. VT switch).
    #[inline]
    pub fn force_redraw(&mut self) {
        self.needs_redraw = true;
        if let Some(p) = &self.redraw_ping { p.ping(); }
    }
    #[inline]
    pub fn take_needs_redraw(&mut self) -> bool { std::mem::replace(&mut self.needs_redraw, false) }
    #[inline]
    pub fn mark_render_queued(&mut self) { self.render_in_flight = true; }
    #[inline]
    pub fn mark_vblank_arrived(&mut self) { self.render_in_flight = false; }
}

// ── SeatHandler for Dispatch (REQUIRED here: `Seat<Dispatch>` field) ──────────
// Inlined from seat.dispatch / seat.focus. `set_data_device_focus` is deferred
// to wire.base via `pending_data_focus` (it needs DataDeviceHandler).
impl SeatHandler for Dispatch {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> { &mut self.seat.state }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.seat.pointer_status = image;
        self.schedule_redraw();
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused
            .and_then(|s| self.output.display_handle.get_client(s.id()).ok());

        if let Some(pointer) = seat.get_pointer() {
            // Deactivate on whatever lost keyboard focus.
            if let Some(old_focus) = self.seat.previous_focus.as_ref().cloned() {
                if let Some(token) = self.seat.deactivate_constraint_for(&old_focus, &pointer) {
                    self.pending_restoration.push(token);
                }
            }
            // Activate on the newly focused surface if the pointer is also over it.
            if let Some(new_focus) = focused {
                if self.seat.is_pointer_over(&pointer, new_focus) {
                    with_pointer_constraint(new_focus, &pointer, |c| {
                        if let Some(c) = c {
                            if !c.is_active() { c.activate(); }
                        }
                    });
                }
            }
            self.seat.previous_focus = focused.cloned();
        }

        // `set_data_device_focus` needs DataDeviceHandler (downstream) — defer it.
        self.pending_data_focus = Some(client);
        self.schedule_redraw();
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, led_state: LedState) {
        // Mirror the xkb NumLock/CapsLock/ScrollLock state onto the physical
        // keyboard LEDs (udev backend). `keyboards` is empty under winit, so
        // this is a no-op there.
        for device in &mut self.seat.keyboards {
            device.led_update(led_state.into());
        }
    }
}

// ── delegate_dispatch2 + ALL smithay handler impls ────────────────────────────
// Every `impl ForeignTrait for Dispatch` (the smithay handler traits, the
// wayland_server `Dispatch`/`GlobalDispatch` from `delegate_dispatch2!`, and the
// color-management protocol impls) can ONLY live in the crate that DEFINES
// `Dispatch` — the orphan rule rejects all of them downstream in wire.base. So
// the handler bodies are INLINED here, depending only on the per-protocol state
// (`*.base`) crates this crate already owns + leaf helpers (no `*.dispatch`
// crate, which would form a cycle). document/SMITHAY_DECOUPLING.md P2 flip.
smithay::delegate_dispatch2!(Dispatch);

mod color_impls {
    use std::sync::Mutex;
    use smithay::reexports::wayland_protocols::wp::color_management::v1::server::{
        wp_color_management_output_v1::{self, WpColorManagementOutputV1},
        wp_color_management_surface_feedback_v1::{self, WpColorManagementSurfaceFeedbackV1},
        wp_color_management_surface_v1::{self, WpColorManagementSurfaceV1},
        wp_color_manager_v1::{self, WpColorManagerV1},
        wp_image_description_creator_icc_v1::{self, WpImageDescriptionCreatorIccV1},
        wp_image_description_creator_params_v1::{self, WpImageDescriptionCreatorParamsV1},
        wp_image_description_info_v1::WpImageDescriptionInfoV1,
        wp_image_description_v1::{self, WpImageDescriptionV1},
    };
    use smithay::reexports::wayland_server::{Client, DataInit, Dispatch as WLDispatch, DisplayHandle, GlobalDispatch, New, Resource};
    use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    use compositor_support_smithay_dispatch_wire_color::color::{ImageDescData, ParamsState};
    use compositor_support_smithay_dispatch_wire_color::color as cm;
    use compositor_support_smithay_dispatch_wire_colorsurf::colorsurf as cs;
    use super::Dispatch;

    impl GlobalDispatch<WpColorManagerV1, ()> for Dispatch {
        fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<WpColorManagerV1>, _: &(), di: &mut DataInit<'_, Self>) {
            cm::bind_color_manager(di.init(resource, ()));
        }
    }
    impl WLDispatch<WpColorManagerV1, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpColorManagerV1, request: wp_color_manager_v1::Request, _: &(), _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            cs::dispatch_color_manager(request, di);
        }
    }
    impl WLDispatch<WpColorManagementOutputV1, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpColorManagementOutputV1, request: wp_color_management_output_v1::Request, _: &(), _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            cs::dispatch_color_output(request, di);
        }
    }
    impl WLDispatch<WpColorManagementSurfaceV1, WlSurface> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpColorManagementSurfaceV1, request: wp_color_management_surface_v1::Request, surface: &WlSurface, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
            cs::dispatch_color_surface(request, surface);
        }
    }
    impl WLDispatch<WpColorManagementSurfaceFeedbackV1, WlSurface> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpColorManagementSurfaceFeedbackV1, request: wp_color_management_surface_feedback_v1::Request, _: &WlSurface, _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            cs::dispatch_color_feedback(request, di);
        }
    }
    impl WLDispatch<WpImageDescriptionCreatorParamsV1, Mutex<ParamsState>> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpImageDescriptionCreatorParamsV1, request: wp_image_description_creator_params_v1::Request, data: &Mutex<ParamsState>, _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            cs::dispatch_color_params(request, data, di);
        }
    }
    impl WLDispatch<WpImageDescriptionCreatorIccV1, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpImageDescriptionCreatorIccV1, request: wp_image_description_creator_icc_v1::Request, _: &(), _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            cs::dispatch_color_icc(request, di);
        }
    }
    impl WLDispatch<WpImageDescriptionV1, ImageDescData> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpImageDescriptionV1, request: wp_image_description_v1::Request, data: &ImageDescData, _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            cs::dispatch_image_desc(request, data, di);
        }
    }
    impl WLDispatch<WpImageDescriptionInfoV1, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpImageDescriptionInfoV1, _: <WpImageDescriptionInfoV1 as Resource>::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
    }
}

// ── Marker impls ───────────────────────────────────────────────────────────────
// `DispatchWire` is local to this crate → impl here. `FactoryBounds` is local to
// state.bounds (which deps on this crate), so `impl FactoryBounds for Dispatch`
// lives THERE to avoid a dependency cycle (document/SMITHAY_DECOUPLING.md P2).
impl DispatchWire for Dispatch {}

// ── All smithay handler impls (inlined; bodies use only `*.base` state +
// smithay + leaf helpers — no `*.dispatch` crate, to avoid a dependency cycle).
mod handler_impls {
    use std::sync::Mutex;
    use smithay::backend::allocator::dmabuf::Dmabuf;
    use smithay::backend::renderer::utils::on_commit_buffer_handler;
    use smithay::desktop::{PopupKind, PopupManager, find_popup_root_surface};
    use smithay::input::{Seat, SeatState};
    use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
    use smithay::input::pointer::{Focus, PointerHandle};
    use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge;
    use smithay::reexports::wayland_server::{Client, Resource};
    use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface};
    use smithay::utils::{Logical, Point, Rectangle, Serial};
    use smithay::wayland::buffer::BufferHandler;
    use smithay::wayland::compositor::{self, CompositorClientState, CompositorHandler, CompositorState, BufferAssignment, SurfaceAttributes, add_blocker, add_pre_commit_hook};
    use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
    use smithay::wayland::drm_syncobj::{DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState};
    use smithay::wayland::fractional_scale::FractionalScaleHandler;
    use smithay::wayland::input_method::InputMethodHandler;
    use smithay::wayland::output::OutputHandler;
    use smithay::wayland::pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint};
    use smithay::wayland::selection::SelectionHandler;
    use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler};
    use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState};
    use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState};
    use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
    use smithay::wayland::shm::{ShmHandler, ShmState};
    use smithay::wayland::tablet_manager::TabletSeatHandler;
    use smithay::wayland::xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData};
    use smithay::wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState};
    use compositor_support_smithay_dispatch_wire_redraw::redraw as rd;
    use compositor_support_smithay_state_xdg_activation_request::{ActivationDetails};
    use compositor_support_smithay_wayland_connection_record::record::WaylandClientSession;
    use super::Dispatch;

    fn unconstrain_popup(popup: &PopupSurface) {
        let infinite_target = Rectangle::from_loc_and_size((-100_000, -100_000), (200_000, 200_000));
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(infinite_target);
        });
    }

    impl PointerConstraintsHandler for Dispatch {
        fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
            let pointer_focused = pointer.current_focus().map(|f| &f == surface).unwrap_or(false);
            if !pointer_focused { return; }
            if !self.seat.is_keyboard_focused(surface) { return; }
            with_pointer_constraint(surface, pointer, |c| { if let Some(c) = c { if !c.is_active() { c.activate(); } } });
        }
        fn remove_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
            if let Some(token) = self.seat.deactivate_constraint_for(surface, pointer) {
                self.pending_restoration.push(token);
            }
        }
        fn cursor_position_hint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>, location: Point<f64, Logical>) {
            with_pointer_constraint(surface, pointer, |c| {
                if c.is_some() { self.seat.unlock_restoration_location = Some((surface.clone(), location)); }
            });
        }
    }

    impl DrmSyncobjHandler for Dispatch {
        fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> { self.dmabuf.syncobj_state.as_mut() }
    }
    impl TabletSeatHandler for Dispatch {}
    impl InputMethodHandler for Dispatch {
        fn new_popup(&mut self, surface: smithay::wayland::input_method::PopupSurface) {
            if let Err(err) = self.popup.state.track_popup(PopupKind::from(surface)) {
                warn!("failed to track input-method popup err={err:?}");
            }
            self.schedule_redraw();
        }
        fn dismiss_popup(&mut self, surface: smithay::wayland::input_method::PopupSurface) {
            // IME-scoped: untrack ONLY this popup from the shared PopupManager, never a
            // blanket clear (xdg popups live in the same manager). smithay calls this from
            // deactivate / re-activate while the popup still holds its (old) parent, so the
            // root surface is resolvable and we remove exactly this node.
            let kind = PopupKind::from(surface);
            if let Ok(root) = find_popup_root_surface(&kind) {
                let _ = PopupManager::dismiss_popup(&root, &kind);
            }
            self.schedule_redraw();
        }
        fn popup_repositioned(&mut self, _surface: smithay::wayland::input_method::PopupSurface) {
            // The popup's location is read live from the surface at render time
            // (`PopupKind::location()` → `set_text_input_rectangle`), so a reposition only
            // needs a repaint — there is no cached position to update here.
            self.schedule_redraw();
        }
        fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
            self.geometries.get(parent).copied().unwrap_or_default()
        }
    }
    impl XdgForeignHandler for Dispatch {
        fn xdg_foreign_state(&mut self) -> &mut XdgForeignState { &mut self.xdg_foreign_state.xdg_foreign_state }
    }
    impl FractionalScaleHandler for Dispatch {
        fn new_fractional_scale(&mut self, surface: WlSurface) {
            let Some(scale) = self.fractional.last_emitted() else { return; };
            rd::new_fractional_scale(scale, &surface);
        }
    }
    impl XdgActivationHandler for Dispatch {
        fn activation_state(&mut self) -> &mut XdgActivationState { &mut self.xdg_activation.xdg_activation }
        fn request_activation(&mut self, token: XdgActivationToken, token_data: XdgActivationTokenData, surface: WlSurface) {
            compositor_support_smithay_state_xdg_activation_request::request_activation(surface, token, token_data);
        }
    }
    impl XdgShellHandler for Dispatch {
        fn xdg_shell_state(&mut self) -> &mut XdgShellState { &mut self.xdg_shell.state }
        fn new_toplevel(&mut self, surface: ToplevelSurface) {
            self.new_toplevels.push(smithay::desktop::Window::new_wayland_window(surface));
            self.schedule_redraw();
        }
        fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
            self.destroyed_toplevels.push(surface);
            self.schedule_redraw();
        }
        fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
            unconstrain_popup(&surface);
            let _ = self.popup.state.track_popup(PopupKind::Xdg(surface));
            self.schedule_redraw();
        }
        fn popup_destroyed(&mut self, _: PopupSurface) { self.schedule_redraw(); }
        fn fullscreen_request(&mut self, surface: ToplevelSurface, _: Option<WlOutput>) {
            self.fullscreen_requests.push((surface, true));
            self.schedule_redraw();
        }
        fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
            self.fullscreen_requests.push((surface, false));
            self.schedule_redraw();
        }
        fn move_request(&mut self, _: ToplevelSurface, _: WlSeat, _: Serial) {}
        fn resize_request(&mut self, _: ToplevelSurface, _: WlSeat, _: Serial, _: ResizeEdge) {}
        fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
            // TODO: popup grabs.
            self.schedule_redraw();
        }
        fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
            surface.with_pending_state(|state| {
                let geometry = positioner.get_geometry();
                state.geometry = geometry;
                state.positioner = positioner;
            });
            unconstrain_popup(&surface);
            surface.send_repositioned(token);
            self.schedule_redraw();
        }
    }
    impl BufferHandler for Dispatch {
        fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
    }
    impl SelectionHandler for Dispatch { type SelectionUserData = (); }
    impl DataDeviceHandler for Dispatch {
        fn data_device_state(&mut self) -> &mut DataDeviceState { &mut self.clipboard.data_device_state }
    }

    fn install_syncobj_blocker_hook(surface: &WlSurface) {
        trace!("installing syncobj blocker hook surface={surface:?}");
        add_pre_commit_hook::<Dispatch, _>(surface, |state: &mut Dispatch, _dh, surface| {
            let maybe_acquire = compositor::with_states(surface, |states| {
                let mut cached = states.cached_state.get::<DrmSyncobjCachedState>();
                cached.pending().acquire_point.clone()
            });
            let Some(acquire) = maybe_acquire else { return; };
            let has_new_buffer = compositor::with_states(surface, |states| {
                let mut cached = states.cached_state.get::<SurfaceAttributes>();
                matches!(cached.pending().buffer, Some(BufferAssignment::NewBuffer(_)))
            });
            if !has_new_buffer { trace!("syncobj: acquire point but no new buffer; skipping blocker"); return; }
            let (blocker, source) = match acquire.generate_blocker() {
                Ok(pair) => pair,
                Err(err) => { warn!("failed to generate syncobj blocker err={err:?}"); return; }
            };
            add_blocker(surface, blocker);
            state.pending_blockers.push((surface.downgrade(), source));
        });
    }

    impl CompositorHandler for Dispatch {
        fn compositor_state(&mut self) -> &mut CompositorState { &mut self.compositor.state }
        fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
            &client.get_data::<WaylandClientSession>().unwrap().compositor_state
        }
        fn new_surface(&mut self, surface: &WlSurface) {
            install_syncobj_blocker_hook(surface);
            let Some(scale) = self.fractional.last_emitted() else { return; };
            rd::new_surface_fractional(scale, surface);
        }
        fn commit(&mut self, surface: &WlSurface) {
            // PROTOCOL only — buffer handling + popup commit; the world effect is
            // applied at drain via `apply_commit` (document/SMITHAY_DECOUPLING.md).
            on_commit_buffer_handler::<Dispatch>(surface);
            self.popup.state.commit(surface);
            if let Some(popup) = self.popup.state.find_popup(surface) {
                match popup {
                    PopupKind::Xdg(ref xdg) => {
                        if !xdg.is_initial_configure_sent() {
                            xdg.send_configure().unwrap_or_else(|e| abort!("initial configure failed: {e:?}"));
                        }
                    }
                    PopupKind::InputMethod(ref _im) => {}
                }
            }
            self.committed.push(surface.clone());
            self.schedule_redraw();
        }
    }
    impl DmabufHandler for Dispatch {
        fn dmabuf_state(&mut self) -> &mut DmabufState { &mut self.dmabuf.state }
        fn dmabuf_imported(&mut self, global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
            self.pending_dmabuf.push((global.clone(), dmabuf, notifier));
        }
    }
    impl DndGrabHandler for Dispatch {
        fn cancelled(&mut self, _: Seat<Self>, _: Point<f64, Logical>) { self.dnd.icon = None; self.schedule_redraw(); }
        fn dropped(&mut self, _: Option<smithay::input::dnd::DndTarget<'_, Self>>, _: bool, _: Seat<Self>, _: Point<f64, Logical>) {
            self.dnd.icon = None; self.schedule_redraw();
        }
    }
    impl WaylandDndGrabHandler for Dispatch {
        fn dnd_requested<Src: Source>(&mut self, source: Src, icon: Option<WlSurface>, seat: Seat<Dispatch>, serial: Serial, type_: GrabType) {
            match type_ {
                GrabType::Pointer => {
                    let Some(ptr) = seat.get_pointer() else { return; };
                    let Some(start_data) = ptr.grab_start_data() else { return; };
                    let grab = DnDGrab::new_pointer(&self.output.display_handle, start_data, source, seat);
                    self.dnd.icon = icon;
                    ptr.set_grab(self, grab, serial, Focus::Keep);
                }
                GrabType::Touch => { source.cancel(); }
            }
        }
    }
    impl WlrLayerShellHandler for Dispatch {
        fn shell_state(&mut self) -> &mut WlrLayerShellState { &mut self.layershell.wlr }
        fn new_layer_surface(&mut self, surface: LayerSurface, output: Option<WlOutput>, layer: Layer, namespace: String) {
            self.new_layers.push((surface, output, layer, namespace));
            self.schedule_redraw();
        }
        fn layer_destroyed(&mut self, surface: LayerSurface) {
            self.destroyed_layers.push(surface);
            self.schedule_redraw();
        }
    }
    impl OutputHandler for Dispatch {}
    impl ShmHandler for Dispatch {
        fn shm_state(&self) -> &ShmState { &self.shm.state }
    }
    impl XdgDecorationHandler for Dispatch {
        fn new_decoration(&mut self, toplevel: ToplevelSurface) {
            toplevel.with_pending_state(|state| { state.decoration_mode = Some(Mode::ServerSide); });
            toplevel.send_pending_configure();
        }
        fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
            toplevel.with_pending_state(|state| { state.decoration_mode = Some(mode); });
            toplevel.send_pending_configure();
        }
        fn unset_mode(&mut self, toplevel: ToplevelSurface) {
            toplevel.with_pending_state(|state| { state.decoration_mode = Some(Mode::ServerSide); });
            toplevel.send_pending_configure();
        }
    }

    // Silence unused-import for SeatState (referenced via SeatHandler in state.rs).
    #[allow(dead_code)]
    fn _seatstate_marker(_: &SeatState<Dispatch>) {}
}
