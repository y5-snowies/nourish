// Wire<S>: the calloop event-loop data.  One per compositor session.
//
// The Wayland dispatch type `D` is NOT `Wire<S>` anymore — it is the concrete,
// non-generic `Dispatch` (document/SMITHAY_DECOUPLING.md → "P2 flip"). ALL the
// smithay handler trait impls live HERE on `Dispatch` (orphan rule: `Dispatch`
// is foreign, but this is the crate the project chose to host the impls, and
// `delegate_dispatch2!(Dispatch)` is co-located so every `GlobalDispatch`/
// `Dispatch` bound is provable here — the factory + the bound-requiring
// dispatcher fns are therefore called from this crate too).
use std::sync::Mutex;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::drm::DrmDeviceFd;
use smithay::desktop::PopupKind;
use smithay::input::dnd::{DndGrabHandler, GrabType, Source};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::input::pointer::{MotionEvent, PointerHandle};
use smithay::reexports::calloop::LoopHandle;
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
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch as WLDispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::drm_syncobj::{DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState};
use smithay::wayland::input_method::InputMethodHandler;
use smithay::wayland::pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::data_device::set_data_device_focus;
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::xdg_foreign::XdgForeignHandler;
use smithay::wayland::compositor;
use smithay::wayland::compositor::{BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes, add_blocker, add_pre_commit_hook};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::fractional_scale::FractionalScaleHandler;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState};
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData};
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};
use compositor_support_smithay_dispatch_state_bounds::FactoryBounds;
use compositor_support_smithay_dispatch_wire_color::color::{ImageDescData, ParamsState};
use compositor_support_smithay_dispatch_wire_trait::wire_trait::WireTrait;
use compositor_support_smithay_dispatch_wire_color::color as cm;
use compositor_support_smithay_dispatch_wire_colorsurf::colorsurf as cs;
use compositor_support_smithay_dispatch_wire_redraw::redraw as rd;

// ── Wire type ────────────────────────────────────────────────────────────────

pub struct Wire<S: WireTrait + 'static> {
    pub inner: S,
    pub state: Dispatch,
    pub loop_handle: LoopHandle<'static, Wire<S>>,
}

impl<A: WireTrait + 'static> Wire<A> {
    pub fn new(
        inner: A,
        display_handle: &DisplayHandle,
        drm_device: Option<DrmDeviceFd>,
        loop_handle: LoopHandle<'static, Wire<A>>,
    ) -> Self {
        let dispatch = new_dispatch(display_handle, drm_device);
        cm::create_global::<Dispatch>(display_handle);
        Self { state: dispatch, inner, loop_handle }
    }
}

// ── Relocated factory ──────────────────────────────────────────────────────────
// `new() -> Dispatch` lives here (NOT in state.new): the sub-factories call
// `create_global::<Dispatch>` for every protocol, which needs
// `Dispatch: GlobalDispatch<…>` — provable only where `delegate_dispatch2!`
// + the handler impls are (this crate). document/SMITHAY_DECOUPLING.md.
pub fn new_dispatch(
    display_handle: &DisplayHandle,
    drm_device: Option<DrmDeviceFd>,
) -> Dispatch {
    Dispatch {
        xdg_activation: compositor_support_smithay_state_xdg_activation_factory::factory::new::<Dispatch>(display_handle),
        dmabuf: compositor_support_smithay_state_dmabuf_factory::factory::new::<Dispatch>(display_handle, drm_device),
        clipboard: compositor_support_smithay_state_clipboard_factory::factory::new::<Dispatch>(display_handle),
        seat: compositor_support_smithay_state_seat_factory::factory::new::<Dispatch>(display_handle),
        xdg_shell: compositor_support_smithay_state_xdg_shell_factory::factory::new::<Dispatch>(display_handle),
        xdg_decoration: compositor_support_smithay_state_xdg_decoration_factory::factory::new::<Dispatch>(display_handle),
        xdg_foreign_state: compositor_support_smithay_state_xdg_foreign_factory::factory::new::<Dispatch>(display_handle),
        shm: compositor_support_smithay_state_shm_factory::factory::new::<Dispatch>(display_handle),
        output: compositor_support_smithay_state_output_factory::factory::new::<Dispatch>(display_handle),
        popup: compositor_support_smithay_state_popup_factory::factory::new::<Dispatch>(),
        layershell: compositor_support_smithay_state_layershell_factory::factory::new::<Dispatch>(display_handle),
        compositor: compositor_support_smithay_state_compositor_factory::factory::new::<Dispatch>(display_handle),
        presentation: compositor_support_smithay_state_presentation_factory::factory::new::<Dispatch>(display_handle),
        viewporter: compositor_support_smithay_state_viewporter_factory::factory::new::<Dispatch>(display_handle),
        fractional: compositor_support_smithay_state_fractional_factory::factory::new::<Dispatch>(display_handle),
        cursor_shape: compositor_support_smithay_state_cursor_shape_factory::factory::new::<Dispatch>(display_handle),
        text_input: compositor_support_smithay_state_text_input_factory::factory::new::<Dispatch>(display_handle),
        dnd: compositor_support_smithay_state_dnd_factory::factory::new(),
        singlepixel: compositor_support_smithay_state_singlepixel_factory::factory::new::<Dispatch>(display_handle),
        needs_redraw: true,
        redraw_ping: None,
        render_in_flight: false,
        committed: vec![],
        new_toplevels: vec![],
        destroyed_toplevels: vec![],
        fullscreen_requests: vec![],
        new_layers: vec![],
        destroyed_layers: vec![],
        pending_dmabuf: vec![],
        geometries: std::collections::HashMap::new(),
        pending_restoration: vec![],
        pending_blockers: vec![],
        pending_data_focus: None,
    }
}

// `delegate_dispatch2!(Dispatch)`, the marker impls (`DispatchWire`,
// `FactoryBounds`), and ALL smithay handler impls live in state.base (the crate
// that DEFINES `Dispatch`) — the orphan rule forbids them here
// (document/SMITHAY_DECOUPLING.md P2 flip). wire.base keeps only the calloop
// `Wire<S>` data, the relocated `Dispatch` factory, and the outbox drain.

// ── Inherent helpers (on Wire<A>: they bridge `state` (seat) + `inner` (world)) ─
impl<A: WireTrait + 'static> Wire<A> {
    #[inline] pub fn schedule_redraw_post_vblank(&mut self) { self.state.schedule_redraw_post_vblank(); }
    #[inline] pub fn schedule_redraw(&mut self) { self.state.schedule_redraw(); }
    #[inline] pub fn force_redraw(&mut self) { self.state.force_redraw(); }
    #[inline] pub fn take_needs_redraw(&mut self) -> bool { self.state.take_needs_redraw() }
    #[inline] pub fn mark_render_queued(&mut self) { self.state.mark_render_queued(); }
    #[inline] pub fn mark_vblank_arrived(&mut self) { self.state.mark_vblank_arrived(); }
    pub fn window_for_toplevel(&self, surface: &ToplevelSurface) -> Option<smithay::desktop::Window> {
        rd::window_for_toplevel(&self.inner.host_space().state, surface.wl_surface())
    }
    pub fn apply_constraint_restoration(&mut self, token: (WlSurface, Point<f64, Logical>)) {
        let (hint_surface, hint_surface_local) = token;
        let Some(pointer) = self.state.seat.seat.get_pointer() else { return; };
        let surface_origin = self.inner.host_space().element_location_for_surface(&hint_surface).to_f64();
        let warp_world = surface_origin + hint_surface_local;
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.state.compositor.clock.now().as_millis() as u32;
        pointer.motion(&mut self.state, Some((hint_surface, hint_surface_local)), &MotionEvent { location: warp_world, serial, time });
        pointer.frame(&mut self.state);
        self.inner.apply_pointer(warp_world);
        self.state.schedule_redraw();
    }

    /// Drain ALL protocol outboxes and apply their world effects against the host
    /// Space (run right after `dispatch_clients`, same iteration, synchronous).
    /// This is the bridge that lets the wayland handlers stay world-free —
    /// document/SMITHAY_DECOUPLING.md.
    pub fn drain_protocol(&mut self) {
        // New toplevels first: initialize data + map so commits can find them.
        for window in std::mem::take(&mut self.state.new_toplevels) {
            self.inner.initialize_surface_data(window.clone());
            self.inner.host_space_mut().state.map_element(window, (0, 0), false);
        }
        // Commits: on_commit, initial configure + placement, resize.
        for surface in std::mem::take(&mut self.state.committed) {
            if let Some((window, geometry)) =
                compositor_support_smithay_state_compositor_dispatch::wire::apply_commit(
                    &mut self.inner.host_space_mut().state,
                    &surface,
                )
            {
                self.inner.place_window(window, geometry);
            }
        }
        // (un)fullscreen.
        for (toplevel, fullscreen) in std::mem::take(&mut self.state.fullscreen_requests) {
            if let Some(w) = self.window_for_toplevel(&toplevel) {
                self.inner.fullscreen_request(w, fullscreen);
            }
        }
        // Layer shell map / unmap.
        for (surface, output, layer, namespace) in std::mem::take(&mut self.state.new_layers) {
            compositor_support_smithay_state_layershell_dispatch::wire::new_layer_surface(
                self.inner.host_space(), surface, output, layer, namespace,
            );
        }
        for surface in std::mem::take(&mut self.state.destroyed_layers) {
            compositor_support_smithay_state_layershell_dispatch::wire::layer_destroyed(
                self.inner.host_space(), surface,
            );
        }
        // Destroyed toplevels.
        for surface in std::mem::take(&mut self.state.destroyed_toplevels) {
            self.inner.destroy_surface_data(surface);
        }
        // Dmabuf imports (GPU binding lives in the kernel; resolves the notifier).
        for (global, dmabuf, notifier) in std::mem::take(&mut self.state.pending_dmabuf) {
            let bound = self.inner.dmabuf_import(&mut self.state, &global, dmabuf, notifier);
            if let Some((dmabuf, notifier)) = bound {
                compositor_support_smithay_state_dmabuf_dispatch::wire::dmabuf_imported::<Dispatch>(
                    &mut self.state, &global, dmabuf, notifier,
                );
            }
        }
        // Syncobj fence sources recorded by the pre-commit hook: insert them now
        // (the hook has no loop_handle; the rim does). When the fence fires, clear
        // the blocker on the client's compositor state + schedule a redraw.
        for (surface_weak, source) in std::mem::take(&mut self.state.pending_blockers) {
            let result = self.loop_handle.insert_source(source, move |_event, _meta, wire| {
                let dh = wire.state.output.display_handle.clone();
                let Ok(surface) = surface_weak.upgrade() else {
                    warn!("blocker Surface destroyed before fence fired."); return Ok(());
                };
                let Some(client) = surface.client() else {
                    warn!("blocker Surface alive but client gone."); return Ok(());
                };
                let client_state = wire.state.client_compositor_state(&client);
                client_state.blocker_cleared(&mut wire.state, &dh);
                // `blocker_cleared` re-applies the held commit (CompositorHandler::
                // commit pushes the surface into `committed`), but only
                // `drain_protocol` turns a commit into its world effects (initial
                // placement → InitialMap → map). The wayland source isn't firing
                // here — the client was blocked on its own render fence and has sent
                // nothing more — so we must drain now, or an explicit-sync client's
                // first buffer (e.g. GTK4 / gnome-calculator) stays unmapped until
                // some unrelated dispatch happens to drain it.
                wire.drain_protocol();
                wire.state.schedule_redraw();
                Ok(())
            });
            if let Err(err) = result { warn!("failed to insert syncobj source err={err:?}"); }
        }
        // Deferred data-device focus (needs DataDeviceHandler, available here).
        if let Some(client) = self.state.pending_data_focus.take() {
            set_data_device_focus(&self.state.output.display_handle, &self.state.seat.seat, client);
        }
        // Pointer-constraint restorations (seat warp + space read).
        for token in std::mem::take(&mut self.state.pending_restoration) {
            self.apply_constraint_restoration(token);
        }
        // Refresh the geometry mirror for synchronous handler reads.
        let geoms: Vec<(WlSurface, smithay::utils::Rectangle<i32, smithay::utils::Logical>)> =
            self.inner.host_space().state.elements()
                .filter_map(|w| w.toplevel().map(|t| (t.wl_surface().clone(), w.geometry())))
                .collect();
        self.state.geometries = geoms.into_iter().collect();
    }
}

// `client_compositor_state` accessor used by the drain (it lives on the handler
// trait `CompositorHandler for Dispatch`, but the inherent borrow path is
// clearer; re-expose the dispatcher fn for the drain).

