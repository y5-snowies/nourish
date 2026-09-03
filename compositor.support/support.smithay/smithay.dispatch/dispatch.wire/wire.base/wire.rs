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
use smithay::xwayland::{X11Surface, xwm::X11Window};
use compositor_support_smithay_state_window_find::find;
use compositor_support_smithay_state_window_ident::ident;
use compositor_support_smithay_state_window_shell::shell;
use compositor_support_smithay_dispatch_state_deferred::deferred::Deferred;
use compositor_support_smithay_dispatch_state_deferred::deferred;
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
use smithay::input::tablet::TabletSeatHandler;
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
use compositor_support_smithay_dispatch_wire_trait::wire_trait::{ActivationOrigin, WireTrait};
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
        compositor_support_smithay_dispatch_wire_tablet::tablet::create_global::<Dispatch>(display_handle);
        // Advertised unconditionally: a client may declare its tearing intent
        // before the user picks a mode, and a bound global cannot be revoked.
        // The hint only ever TAGS a surface; whether it tears is the
        // compositor's call (`environment.tearing`).
        compositor_support_smithay_dispatch_wire_tearing::tearing::create_global::<Dispatch>(display_handle);
        // `xdg_toplevel_icon_v1`: the client-declared icon. Read off the surface's
        // cached state by the introspection extraction (see `Meta::xdg_icon_name`),
        // so there is no per-surface state to keep here.
        compositor_support_smithay_dispatch_wire_icon::icon::create_global::<Dispatch>(display_handle);
        // `xdg_session_management_v1`: lets a client declare durable identity for
        // its toplevels instead of us inferring it after the fact. Advertised
        // unconditionally — a client that never binds it is unaffected, and the
        // placeholder path treats the identity as an ADDITIONAL signal on top of
        // the activation token / pid tree, never a replacement.
        compositor_support_smithay_dispatch_wire_session::session::create_global::<Dispatch>(display_handle);
        // `xdg_toplevel_drag_v1` — browsers bind this to detach a tab into its
        // own window, and Chromium/Firefox only enable that UI when the global
        // is present, so it is advertised unconditionally like the rest.
        compositor_support_smithay_dispatch_wire_drag::drag::create_global::<Dispatch>(display_handle);
        // Both namespaces are advertised: `xdg_` is the current staging name,
        // `xx_` is the pre-rename one GTK 4.22 actually binds — and today GTK is
        // the only shipping client, so without this no real app reaches the
        // feature at all. A client binds whichever it knows; both land in the
        // same store, so the placeholder path cannot tell them apart.
        compositor_support_smithay_dispatch_wire_session::session::create_legacy_global::<Dispatch>(display_handle);
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
        xdg_dialog: compositor_support_smithay_state_xdg_dialog_factory::factory::new::<Dispatch>(display_handle),
        shm: compositor_support_smithay_state_shm_factory::factory::new::<Dispatch>(display_handle),
        output: compositor_support_smithay_state_output_factory::factory::new::<Dispatch>(display_handle),
        popup: compositor_support_smithay_state_popup_factory::factory::new::<Dispatch>(),
        layershell: compositor_support_smithay_state_layershell_factory::factory::new::<Dispatch>(display_handle),
        foreign: {
            // `protocol_foreign` preference (preferences.json) startup snapshot: when
            // disabled, NEITHER foreign-toplevel global is advertised (clients can't bind
            // them); when enabled, both are. Read once at boot — no hot-reload; a change
            // takes effect on the next launch.
            let prefs = compositor_model_environment_preference_base::base::load();
            let enabled = prefs.protocol_foreign == "enabled";
            let all_worlds = prefs.protocol_foreign_all_worlds;
            compositor_support_smithay_state_foreign_factory::factory::new::<Dispatch>(display_handle, enabled, all_worlds)
        },
        compositor: compositor_support_smithay_state_compositor_factory::factory::new::<Dispatch>(display_handle),
        presentation: compositor_support_smithay_state_presentation_factory::factory::new::<Dispatch>(display_handle),
        viewporter: compositor_support_smithay_state_viewporter_factory::factory::new::<Dispatch>(display_handle),
        fractional: compositor_support_smithay_state_fractional_factory::factory::new::<Dispatch>(display_handle),
        cursor_shape: compositor_support_smithay_state_cursor_shape_factory::factory::new::<Dispatch>(display_handle),
        text_input: compositor_support_smithay_state_text_input_factory::factory::new::<Dispatch>(display_handle),
        dnd: compositor_support_smithay_state_dnd_factory::factory::new(),
        singlepixel: compositor_support_smithay_state_singlepixel_factory::factory::new::<Dispatch>(display_handle),
        tablet: Default::default(),
        session: Default::default(),
        session_live: Default::default(),
        toplevel_drag: Default::default(),
        redraw: compositor_support_smithay_state_redraw_schedule::schedule::Schedule::new(),
        xwayland: compositor_support_smithay_state_xwayland_factory::factory::new::<Dispatch>(display_handle),
        protocol_pending: false,
        committed: vec![],
        deferred: vec![],
        pending_dmabuf: vec![],
        geometries: std::collections::HashMap::new(),
        outputs_snapshot: vec![],
        in_popup_grab: false,
        pending_constraint_activation: None,
        pending_blockers: vec![],
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
    #[inline] pub fn bump_redraw_epoch(&mut self) { self.state.bump_redraw_epoch(); }
    #[inline] pub fn schedule_redraw(&mut self) { self.state.schedule_redraw(); }
    #[inline] pub fn force_redraw(&mut self) { self.state.force_redraw(); }
    /// Apply a control request a dock sent through wlr-foreign-toplevel-management.
    /// `close` asks the client to close; `fullscreen` routes through the world; `activate`
    /// queues a view+activate (see `request_activation`). maximize/minimize have no y5 model
    /// and are dropped at the protocol layer, so they never reach here.
    fn apply_foreign_request(
        &mut self,
        surface: WlSurface,
        request: compositor_support_smithay_state_foreign_base::base::ForeignRequest,
    ) {
        use compositor_support_smithay_state_foreign_base::base::ForeignRequest;
        // Search EVERY world, not just the hosted one — with `all_worlds` a dock can send a
        // request for a window on another world (cross-world activate). `request_activation`
        // downstream switches to that window's world before framing it.
        let Some(window) = self
            .inner
            .all_world_spaces()
            .iter()
            .flat_map(|s| s.state.elements())
            .find(|w| find::is_surface(w, &surface))
            .cloned()
        else {
            return;
        };

        match request {
            ForeignRequest::Close => {
                shell::close(&window);
            }
            ForeignRequest::Fullscreen(fs) => {
                self.inner.fullscreen_request(window, fs);
            }
            ForeignRequest::Activate => {
                // Queue a view+activate; the window-lifecycle drainer (higher crate, has the
                // camera `view`) applies it. `Foreign` records the source for later.
                self.inner.request_activation(window, ActivationOrigin::Foreign);
            }
        }
    }

    /// Re-advertise foreign-toplevels after a world switch: diff the now-hosted world's
    /// Space against the mirror (closing the old world's toplevels, announcing the new
    /// world's). Event-driven — the loader registers this on the `WORLD_SWITCHED` bus
    /// channel, so it runs once per actual switch, not on a per-iteration poll.
    pub fn reconcile_foreign_on_world_change(&mut self) {
        self.foreign_reconcile();
    }

    /// The rim's full response to a `WORLD_SWITCHED` event, in order: cancel a
    /// toplevel drag that was carrying a window (it must go first — tearing down its
    /// pointer grab moves focus itself), put the cursor back where the hand left it,
    /// carry keyboard focus + `activated` to the incoming world (this sets the new
    /// activation), then re-advertise the foreign-toplevel mirror against it. Kept
    /// together here so the composition root only wires an opaque "on world switched"
    /// and stays agnostic of which concerns (drag, pointer, focus, docks) react to a
    /// switch.
    pub fn on_world_switched(&mut self) {
        self.abort_toplevel_drag();
        self.apply_world_switch_pointer();
        self.apply_world_switch_focus();
        self.foreign_reconcile();
    }

    /// Keep the cursor visually still across a world switch.
    ///
    /// The seat pointer's location is a single global y5-WORLD point while every
    /// world has its own camera, so carrying it across a switch draws the cursor
    /// wherever the incoming camera happens to project it — for a world never
    /// entered, whose camera sits at the default, that is far outside the panel.
    /// The hardware position is what actually survives (the Orchestrator hands it to
    /// the incoming world in `set_spawn_target_world`), so `reanchor_pointer`
    /// re-derives the world point from IT under the incoming camera and this states
    /// the result on the seat. The same reprojection the navigator does to hold the
    /// cursor still while the camera eases (`navigator.tick/tick.warp::warp_intent`).
    ///
    /// Focus is dropped (`None`): the surface under the cursor belonged to the world
    /// just left and is owed its `leave`; the next motion enters whatever is here.
    ///
    /// Runs from the `WORLD_SWITCHED` handler, which the frame hook drains BEFORE
    /// `reconcile_finger_pan` — so that pass sees an accumulator and a seat location
    /// that already agree, and leaves both alone.
    pub fn apply_world_switch_pointer(&mut self) {
        let Some(pointer) = self.state.seat.seat.get_pointer() else {
            return;
        };
        let location = self.inner.reanchor_pointer();
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.state.compositor.clock.now().as_millis() as u32;
        pointer.motion(&mut self.state, None, &MotionEvent { location, serial, time });
        pointer.frame(&mut self.state);
    }

    /// End an `xdg_toplevel_drag_v1` that was carrying a window when the world
    /// changed under it.
    ///
    /// A carry has no coherent meaning across a switch: the window stays in the
    /// world it was torn off in, while the drop target, the DnD focus and the
    /// placeholder record all belong to a world that is no longer on screen.
    /// Cancelling is also what the client is prepared for — the spec has it delete
    /// a newly created toplevel on `cancelled`, which is exactly right for a tab
    /// whose destination just vanished.
    ///
    /// Runs FIRST, before focus is carried across, because tearing down a pointer
    /// grab moves focus itself and the drag should be dismantled against the state
    /// it was started in.
    ///
    /// `unset_grab` reaches `DnDGrab::unset`, which routes on `should_drop` — set
    /// only by a physical button release. This path therefore lands on `cancel()`
    /// and can never emit `dnd_drop_performed`, which is owed to real drops alone.
    fn abort_toplevel_drag(&mut self) {
        let Some(carried) = self.state.toplevel_drag.carried_surface() else {
            return;
        };
        let Some(pointer) = self.state.seat.seat.get_pointer() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.state.compositor.clock.now().as_millis() as u32;
        info!("toplevel drag: world switched mid-carry — cancelling the drag");
        // Both of these are recorded BEFORE cancelling, while the carry is still a
        // fact. This path is the mirror of a physical drop: there the inner
        // `DnDGrab` unsets itself first, so `dropped` still sees a live carry and
        // reads it straight out of the handler. Here the pointer unsets the
        // INSTALLED grab, whose `unset` clears `active` before `cancelled` runs —
        // so the handler read comes back empty and the facts must be taken here.
        //
        // The settle keeps the window's placeholder at the position the carry left
        // it at, for a client that holds on to the window.
        self.state.toplevel_drag.settled.push(carried.clone());
        self.state.arm_drain();
        // The abandon verdict is for a client that does the usual thing instead
        // and deletes the toplevel: the destroy then arrives after the carry has
        // ended, too late to ask whether it was being carried.
        self.state.toplevel_drag.abandoned.clear();
        self.state.toplevel_drag.abandoned.push(carried);
        pointer.unset_grab(&mut self.state, serial, time);
    }

    /// Reconcile the foreign-toplevel mirror against the space(s) it advertises: just the
    /// hosted world, or EVERY world when `protocol_foreign_all_worlds` is set. Shared by the
    /// per-commit drain and the world-switch handler.
    pub fn foreign_reconcile(&mut self) {
        if !self.state.foreign.enabled() {
            return;
        }
        if self.state.foreign.all_worlds() {
            let states = self.inner.all_world_spaces();
            let spaces: Vec<_> = states.iter().map(|s| &s.state).collect();
            self.state.foreign.reconcile::<Dispatch>(&spaces);
        } else {
            self.state.foreign.reconcile::<Dispatch>(&[&self.inner.host_space().state]);
        }
    }

    /// Carry keyboard focus + `activated` across a world switch. Keyboard focus is a
    /// single global on the seat, so switching worlds otherwise strands focus (and typed
    /// input) on the outgoing world's window. Runs from the `WORLD_SWITCHED` rim handler
    /// (post-switch: focus is still the outgoing window, `spawn_target` is the incoming
    /// world). Saves the outgoing world's focus, then restores the incoming world's
    /// remembered window (or clears when it has none) — the seat plumbing lives here; the
    /// world/memory/activation logic is behind `WireTrait`. Restoring the incoming focus
    /// transitively pulls the global focus off the outgoing window.
    pub fn apply_world_switch_focus(&mut self) {
        let Some(keyboard) = self.state.seat.seat.get_keyboard() else {
            return;
        };
        // Save: the surface that still holds global focus belongs to the world we just
        // left (disable does not move windows), so it is keyed under the outgoing world.
        if let Some(surface) = keyboard.current_focus() {
            self.inner.remember_focus_of(&surface);
        }
        // Restore: the incoming world's remembered window (activates it too), or `None`
        // to clear focus when it has no live remembered window.
        let focus = self.inner.restore_focus_for_current_world();
        let serial = SERIAL_COUNTER.next_serial();
        keyboard.set_focus(&mut self.state, focus, serial);
    }

    pub fn apply_constraint_restoration(&mut self, token: (WlSurface, Point<f64, Logical>)) {
        let (hint_surface, hint_surface_local) = token;
        let Some(pointer) = self.state.seat.seat.get_pointer() else { return; };
        // The hint is surface-local; where it SHOWS is through the window's fit
        // (world side). Fall back to the raw geometry origin only for a surface no
        // window owns.
        let warp_world = self
            .inner
            .surface_point_to_world(&hint_surface, hint_surface_local)
            .unwrap_or_else(|| {
                self.inner.host_space().element_location_for_surface(&hint_surface).to_f64()
                    + hint_surface_local
            });
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.state.compositor.clock.now().as_millis() as u32;
        // Smithay derives the client's local coordinate as `location - focus_origin`,
        // so hand it the origin that yields exactly the hint (`hit.rs` does the same).
        pointer.motion(&mut self.state, Some((hint_surface, warp_world - hint_surface_local)), &MotionEvent { location: warp_world, serial, time });
        pointer.frame(&mut self.state);
        self.inner.apply_pointer(warp_world);
        self.state.schedule_redraw();
    }

    /// Drain ALL protocol outboxes and apply their world effects against the host
    /// Space (run right after `dispatch_clients`, same iteration, synchronous).
    /// This is the bridge that lets the wayland handlers stay world-free —
    /// document/SMITHAY_DECOUPLING.md.
    pub fn drain_protocol(&mut self) {
        // Consumed, not merely read: whoever drains satisfies the marker, so the loop's
        // end-of-iteration drain skips an iteration the syncobj flush already emptied.
        self.state.protocol_pending = false;
        // Deferred world effects, in ARRIVAL order — one pass, not one loop per kind.
        //
        // Placed here, ahead of `committed` below, because that is the one ordering
        // this queue cannot express: a commit has to find its window already mapped,
        // and the two live in different queues. Everything WITHIN this pass is ordered
        // by when it happened rather than by which arm was written first.
        //
        // Still ahead of `foreign_reconcile` further down, which is what keeps a
        // destroyed window out of the dock in the same frame it died.
        for event in std::mem::take(&mut self.state.deferred) {
            match event {
        // A window asked to be mapped. X11 windows arrive here too
        // (`XwmHandler::map_window_request` and `mapped_override_redirect_window`), so
        // this one arm gives all three kinds their uuid, their Space slot and
        // everything keyed off those.
        Deferred::WindowMapped(mapped) => {
            // Resolve the window's IDENTITY before anything else touches it. For X11 that
            // means asking the Space, which is the authority: an `X11Surface` may be
            // reported as mapped more than once (see `Mapped`), and constructing here
            // would mint a second identity for a window already on the canvas. An
            // already-mapped window is not an error — its placement, uuid and slot are
            // already correct, so there is nothing further to apply. A re-map after an
            // unmap is exactly this case: the unmap kept the element
            // (`XwmHandler::unmapped_window`), so the window resumes its slot, uuid and
            // position with a fresh surface, as an xdg toplevel does.
            //
            // Across every world, because the unmap left the element in its own — a
            // host-only lookup would mint a second identity for a remap off-screen.
            //
            // A tracked POPUP is not a Space element, so the Space cannot answer for it;
            // its surface can. Without this, a map and its association arriving in the
            // same drain minted a second popup node for one menu.
            let window = match mapped {
                deferred::Mapped::Xdg(window) => window,
                deferred::Mapped::X11(x11) => match self.inner.owning_x11_window(&x11) {
                    // Already a live Space element: a second entry for one map, or a
                    // window that mapped anyway. Its placement, uuid and slot are correct.
                    Some((_, window)) if self.inner.is_space_element(&window) => continue,
                    // WITHDRAWN and mapping again. Put it back where it left, under the
                    // uuid it left with, and retire the placeholder that stood in for it.
                    // This is the case that makes a Wine/SDL fullscreen toggle and a
                    // GTK/Qt hide()/show() land where they were instead of coming back as
                    // a new window in the middle of the camera.
                    Some((_, window)) => {
                        self.inner.readmit_x11(window);
                        continue;
                    }
                    None => {
                        let tracked = x11
                            .wl_surface()
                            .is_some_and(|s| self.state.popup.state.find_popup(&s).is_some());
                        if tracked {
                            continue;
                        }
                        smithay::desktop::Window::new_x11_window(x11)
                    }
                },
            };
            // An EPHEMERAL X11 window that names a parent is a menu, tooltip or drag
            // icon, and becomes a popup rather than a window — see `child::as_popup`.
            // Tracked and nothing else: it gets no uuid, no Space slot, no decoration
            // and no placeholder, and is drawn and hit-tested through the paths that
            // already walk `popups_for_surface`. Anything that does not qualify falls
            // through to the window path below, which is still the right answer for a
            // transient dialog and the only one available to a menu that sets no
            // `WM_TRANSIENT_FOR`.
            // HELD until Xwayland associates a wl_surface. `child::as_popup` cannot answer
            // without one — a `PopupKind` carries the popup's surface and its parent's —
            // and Xwayland associates only after the map request, so asking here would
            // answer `None` for EVERY X11 window and no menu, tooltip or dropdown could
            // ever be a popup. Only CANDIDATES wait; an ordinary window maps immediately,
            // which the placement below depends on. The map handlers skip a candidate
            // that has no surface yet (`queue_x11_map`) and `surface_associated` queues
            // it once it does; this check is the safety net for a surface that vanished
            // in between, and the identity resolution above makes a second entry safe.
            if window.is_x11()
                && window.wl_surface().is_none()
                && ident::is_popup_x11(&window)
            {
                continue;
            }
            if window.is_x11() {
                // Every world's windows, not the focused one's. The parent mapped into
                // whichever world was focused when IT mapped, and this child maps into
                // whichever is focused now — so an app that opens a menu while the user
                // is looking elsewhere resolved no parent, and the failure is not a
                // misplacement but a change of KIND: the menu became a window, with a
                // uuid, a Space slot, a decoration and a placeholder.
                //
                // Widening cannot mis-resolve: `parent_offset` matches on `window_id()`,
                // which is unique across the X server, so more candidates can only find
                // the right parent or none. And a popup needs no world of its own —
                // `PopupManager` is world-free and the draw and hit paths reach it by
                // walking `popups_for_surface` from the parent, so one whose parent is in
                // another world is tracked once and drawn there.
                //
                // Same shape as `Deferred::WindowFullscreen` below, for the same reason.
                let mapped: Vec<smithay::desktop::Window> = self
                    .inner
                    .all_world_spaces()
                    .iter()
                    .flat_map(|s| s.state.elements())
                    .cloned()
                    .collect();
                // The fallback parent for a menu that names none: the window the user was
                // last pointing at, then the last focused one.
                let fallback = self
                    .state
                    .xwayland
                    .map_position_parent_hover
                    .or(self.state.xwayland.map_position_parent_focus);
                // The popup registry is the SECOND place a parent can live, and the only
                // one that can answer for a submenu: its parent is the menu, which y5
                // deliberately keeps out of the Space.
                let popups = &self.state.popup.state;
                if let Some(popup) = compositor_support_smithay_state_window_child::child::as_popup(
                    mapped.iter(),
                    &window,
                    fallback,
                    self.state.outputs_snapshot.iter().map(|(_, geometry)| geometry.size),
                    |id| popups.find_x11_popup(id),
                ) {
                    match self.state.popup.state.track_popup(popup) {
                        Ok(()) => {
                            self.state.schedule_redraw();
                            continue;
                        }
                        // The parent died between the map request and this drain. Fall
                        // through: as a window it is at least reachable and closable,
                        // where an untracked popup would be neither drawn nor destroyed.
                        Err(err) => warn!("x11 popup track failed, mapping as a window: {err:?}"),
                    }
                } else if ident::is_popup_x11(&window)
                    && !ident::states(&window).fullscreen
                {
                    // `as_popup` declines for more than `is_popup_x11` can see: fullscreen,
                    // an output-sized override-redirect window that names no parent, a
                    // parent that resolves to nothing, and a `wl_surface` missing on either
                    // side. Only the last two are worth hearing about — the window path
                    // then centres the thing on the CAMERA, so a menu lands mid-view
                    // instead of beside its owner.
                    //
                    // Fullscreen is filtered out here because it is the COMMON path, not a
                    // problem: a fullscreen game is override-redirect, so `is_popup_x11`
                    // says yes and the size guard in `as_popup` says no. Reporting that as
                    // a parenting failure warned on every game launch.
                    warn!("x11 popup candidate has no resolvable parent, mapping as a window");
                }
            }
            self.inner.initialize_surface_data(window.clone());
            self.inner.host_space_mut().state.map_element(window.clone(), (0, 0), false);
            // ...but the commit-driven initial placement below cannot serve an X11
            // window. An X client sizes itself BEFORE asking to be mapped, and the
            // wl_surface Xwayland backs it with may have been committing since before
            // this `Window` existed — so waiting for "the first commit that finds a
            // window with a non-degenerate geometry" can wait forever. The geometry is
            // already final here, so place it now and mark it placed.
            if window.is_x11() {
                let mut geometry = window.geometry();
                geometry.size = shell::configured_size(&window);
                window
                    .user_data()
                    .insert_if_missing(|| compositor_support_smithay_state_compositor_place::WindowPlacedMarker);
                self.inner.place_window(window, geometry);
            }
        }
        Deferred::WindowFullscreen { window, on } => {
            let spaces: Vec<smithay::desktop::Window> =
                self.inner.all_world_spaces().iter().flat_map(|s| s.state.elements()).cloned().collect();
            if let Some(w) = find::window_of(spaces.iter(), &window) {
                self.inner.fullscreen_request(w, on);
            }
        }
        // Layer shell map / unmap. A NULL-output surface goes to the monitor the
        // user is on (active_output), not always the first output.
        Deferred::LayerMapped { surface, output, layer, namespace } => {
            let current_output = self.inner.active_output();
            compositor_support_smithay_state_layershell_dispatch::wire::new_layer_surface(
                self.inner.host_space(), surface, output, layer, namespace, current_output,
            );
        }
        Deferred::LayerDestroyed(surface) => {
            compositor_support_smithay_state_layershell_dispatch::wire::layer_destroyed(
                self.inner.host_space(), surface,
            );
        }
        // The two teardowns differ in shape, not in timing: an xdg toplevel is keyed by
        // its surface (the uuid lives in that surface's data map), while an X11 window
        // is keyed by the window itself — its surface association may already be gone by
        // the time the X server says it died — and has to be unmapped from its Space
        // here, which the xdg path leaves to `refresh_alive`.
        // Out of the Space, identity kept. The Space is the truth about what windows
        // exist, and a withdrawn window is not one of them — it has no surface, is not
        // drawn and is not hit. What survives is the record `withdraw_x11` parks, so a
        // remap resolves the same uuid, world and position.
        Deferred::WindowWithdrawn(surface) => {
            if let Some((world, window)) = self.inner.owning_x11_window(&surface) {
                // Guard against a second withdrawal for one window: the record already
                // holds it, so `owning_x11_window` answered from there and the Space has
                // nothing left to unmap.
                if self.inner.is_space_element(&window) {
                    // NOT unmapped first: `withdraw_x11` reads the window's location out
                    // of the Space to park it, and an element that has already been
                    // unmapped has no location to read — it came back at (0,0), which in
                    // centre-anchored world coordinates is nowhere near where it left.
                    // The window remapped correctly and landed off-camera, which reads as
                    // "it never came back".
                    //
                    // An EPHEMERAL window is RETIRED, not withdrawn: no record, so a
                    // re-show is a fresh map that resolves its parent again.
                    //
                    // Identity across a hide is for windows the user arranged — a Wine
                    // fullscreen toggle, a GTK hide()/show() — where coming back anywhere
                    // else is the bug. A menu is the opposite: its position belongs to
                    // whatever it is opening off, and restoring the slot it had last time
                    // makes the stored location a CACHE. Unity's submenus showed it —
                    // hover away, hover back, and the submenu reappeared where it was
                    // rather than beside the item that opened it.
                    if compositor_support_smithay_state_window_ident::ident::is_ephemeral_x11(&window) {
                        self.inner.space_of_world_mut(world).state.unmap_elem(&window);
                        self.inner.destroy_x11_data(window);
                    } else {
                        // Takes the location, then unmaps, in that order.
                        self.inner.withdraw_x11(world, window);
                    }
                }
            }
        }
        Deferred::WindowDestroyed { window, drag_discard } => match window {
            find::Shell::Xdg(surface) => self.inner.destroy_surface_data(surface, drag_discard),
            find::Shell::X11(surface) => {
                if let Some((world, window)) = self.inner.owning_x11_window(&surface) {
                    // A window that WITHDREW first is already out of the Space and has
                    // already left its placeholder, so the teardown would run twice.
                    // Dropping its record is the whole job: the placeholder stops being
                    // one a remap could reclaim and becomes an ordinary launch
                    // placeholder, which is exactly what a destroy means.
                    if self.inner.is_space_element(&window) {
                        self.inner.space_of_world_mut(world).state.unmap_elem(&window);
                        self.inner.destroy_x11_data(window);
                    }
                    self.inner.forget_withdrawn_x11(&surface);
                }
            }
        },
        // Pointer-constraint restorations (seat warp + space read).
        Deferred::PointerRestore { surface, at } => {
            self.apply_constraint_restoration((surface, at));
        }
            }
        }
        // Commits: on_commit, initial configure + placement, resize.
        let committed = std::mem::take(&mut self.state.committed);
        for surface in &committed {
            // Against the Space of the world that OWNS the window — a window in a
            // world the user is not in still commits, and its `on_commit` (bbox) and
            // startup jiggle must land there, not be dropped because it is absent
            // from the host Space. A not-yet-mapped toplevel resolves to the host
            // Space, where `new_toplevels` above just mapped it.
            //
            // The remembered size only for a MAPPED toplevel's commit preceding its
            // initial configure — the one moment it can still be proposed. Commits
            // are the hottest path here, and that lookup walks every world's
            // placeholders. Resolved BEFORE the mutable space borrow: the
            // placeholder that answers lives in the world slot.
            use compositor_support_smithay_state_compositor_dispatch::wire as commit;
            let committed = find::in_space(&self.inner.owning_space(surface).state, surface);
            let initial = committed.is_some() && commit::awaits_initial_configure(surface);
            let restore_size = initial.then(|| self.inner.session_restore_size(surface)).flatten();
            if let Some((window, geometry)) = commit::apply_commit(
                &mut self.inner.owning_space_mut(surface).state,
                surface,
                committed,
                initial,
                restore_size,
            )
            {
                self.inner.place_window(window, geometry);
            }
        }
        // Live layer-shell reconfiguration: if a committed surface is a mapped layer
        // surface, re-arrange its output so anchor / size / margin / exclusive-zone
        // changes take effect (and reserved space updates) without a remap.
        let mut layer_relayout = false;
        for surface in &committed {
            if compositor_support_smithay_state_layershell_dispatch::wire::arrange_on_commit(
                self.inner.host_space(),
                surface,
            ) {
                layer_relayout = true;
            }
        }
        if layer_relayout {
            self.state.schedule_redraw();
        }
        // Mirror the current outputs (+ logical geometry) for the world-free layer
        // popup constrain (see `Dispatch::outputs_snapshot`). Cheap; a step behind by
        // one iteration, which is fine since outputs change only on hotplug.
        let outputs_snapshot = {
            let space = &self.inner.host_space().state;
            space
                .outputs()
                .filter_map(|o| space.output_geometry(o).map(|g| (o.clone(), g)))
                .collect()
        };
        self.state.outputs_snapshot = outputs_snapshot;
        // Deferred pointer-constraint activate-on-focus (recorded by `focus_changed`).
        // Done here — NOT in the callback — because the pointer is unlocked now, so the
        // `is_pointer_over` → `current_focus()` query can't re-lock a held pointer mutex.
        if let Some(surface) = self.state.pending_constraint_activation.take() {
            if let Some(pointer) = self.state.seat.seat.get_pointer() {
                if self.state.seat.is_pointer_over(&pointer, &surface) && !self.state.seat.constraints_suspended {
                    with_pointer_constraint(&surface, &pointer, |c| {
                        if let Some(c) = c {
                            if !c.is_active() {
                                c.activate();
                            }
                        }
                    });
                }
            }
        }
        // Destroyed windows, xdg and X11 in the order they arrived — and BEFORE
        // `foreign_reconcile` below, so the dock mirror publishes this frame's removals
        // rather than last frame's.
        //
        // The two teardowns differ in shape, not in timing: an xdg toplevel is keyed by
        // its surface (the uuid lives in that surface's data map), while an X11 window
        // is keyed by the window itself — its surface association may already be gone by
        // the time the X server says it died — and has to be unmapped from its Space
        // here, which the xdg path leaves to `refresh_alive`.
        // wlr foreign-toplevel-management: reconcile the dock-facing mirror against
        // the now-updated Space(s) (announce new toplevels, close gone ones, push
        // title/app_id/state deltas), then apply any control requests docks queued.
        self.foreign_reconcile();
        for (surface, request) in self.state.foreign.take_requests() {
            self.apply_foreign_request(surface, request);
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
                // placement → InitialMap → map).
                //
                // Flushed here rather than left to the loop's end-of-iteration drain,
                // which now runs unconditionally and would reach it in this same
                // iteration anyway. What is left is ordering WITHIN the iteration: a
                // render is ping-driven and a ping raised earlier can be dispatched
                // after this source, so leaving the map to the tail can hand that render
                // pre-drain state and cost the client's first buffer a frame. Doing it
                // here puts the map ahead of anything else this dispatch runs.
                //
                // Ordering between queues is unaffected: this takes whatever is queued
                // so far, in arrival order, and the end-of-iteration drain takes the rest.
                wire.drain_protocol();
                wire.state.schedule_redraw();
                Ok(())
            });
            if let Err(err) = result { warn!("failed to insert syncobj source err={err:?}"); }
        }
        // Deferred data-device focus (needs DataDeviceHandler, available here).
        if let Some(client) = self.state.clipboard.pending_focus.take() {
            set_data_device_focus(&self.state.output.display_handle, &self.state.seat.seat, client);
        }
        // Clipboard persistence: start reading a selection the client just set, retire
        // finished readers, write persisted flavors back to pasting clients. All of it
        // needs the `loop_handle` the handlers have no access to. Note this runs right
        // after `dispatch_clients`, so a reader abandoned inside
        // `selection_source_destroyed` is unregistered in the same iteration.
        compositor_support_smithay_dispatch_wire_clipboard::clipboard::drain(
            &mut self.state,
            &self.loop_handle,
            |wire: &mut Wire<A>| &mut wire.state,
        );
        // `xdg_toplevel_drag_v1` drops: queue the placeholder re-sync so it lands
        // ORDERED behind this same drain's `InitialMap`. A tab torn off and dropped
        // inside one frame produces both, and the record the settle needs does not
        // exist until the map ahead of it has been applied.
        //
        // The carried MOVES are deliberately not applied here — a carried window is
        // only ever observed at render, so the frame hook applies the latest queued
        // position once per frame instead (window.lifecycle/lifecycle.interface).
        for surface in std::mem::take(&mut self.state.toplevel_drag.settled) {
            self.inner.settle_toplevel_drag(surface);
        }
        // No X11 stacking sync. It used to publish y5's canvas order here every drain,
        // and that is precisely what undid `Dispatch::raise_x11_for_pointer` within a
        // frame — the X stack is not a mirror of the canvas, it is the mechanism that
        // decides which X client an ungrabbed pointer event reaches, and the pointer is
        // the only thing entitled to move it.
        // Refresh the geometry mirror for synchronous handler reads.
        let geoms: Vec<(WlSurface, smithay::utils::Rectangle<i32, smithay::utils::Logical>)> =
            self.inner.host_space().state.elements()
                .filter_map(|w| ident::surface(w).map(|s| (s, w.geometry())))
                .collect();
        self.state.geometries = geoms.into_iter().collect();
    }
}

// `client_compositor_state` accessor used by the drain (it lives on the handler
// trait `CompositorHandler for Dispatch`, but the inherent borrow path is
// clearer; re-expose the dispatcher fn for the drain).

