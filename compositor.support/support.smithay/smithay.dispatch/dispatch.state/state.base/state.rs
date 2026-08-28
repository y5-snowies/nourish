use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::input::keyboard::LedState;
use smithay::input::dnd::DndGrabHandler;
use smithay::input::pointer::CursorImageStatus;
use smithay::reexports::calloop::{self, LoopHandle};
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::Weak;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
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
    pub xdg_dialog: compositor_support_smithay_state_xdg_dialog_base::state::Dialog,
    pub shm: compositor_support_smithay_state_shm_base::state::SHMState,
    pub output: compositor_support_smithay_state_output_base::state::OutputState,
    pub popup: compositor_support_smithay_state_popup_base::state::PopupState,
    pub layershell: compositor_support_smithay_state_layershell_base::state::Layershell,
    pub foreign: compositor_support_smithay_state_foreign_base::base::ForeignToplevel,
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
    /// External `zwp_tablet_manager_v2` state (tool + pad). Hand-rolled — smithay
    /// hides its tablet-seat instances and has no pad support (see `mod tablet_impls`).
    pub tablet: compositor_support_smithay_dispatch_wire_tablet::tablet::TabletState,
    /// External `xdg_session_management_v1` store: which toplevel names exist in
    /// which session id (see `mod session_impls`). Only the identity index lives
    /// here — the geometry a session restores to is the placeholder's, and the
    /// durable copy of the identity is persisted with it.
    pub session: compositor_support_smithay_state_session_store::store::SessionStore,
    /// Which protocol object currently manages which session id — the
    /// `replaced` / `in_use` half of session management (see `mod session_impls`).
    pub session_live: compositor_support_smithay_dispatch_wire_session::session::SessionLive,
    /// `xdg_toplevel_drag_v1`: which `wl_data_source` owns which drag object.
    pub toplevel_drag: compositor_support_smithay_dispatch_wire_drag::drag::ToplevelDragState,
    /// Redraw scheduling, per pipe: request epoch, each output's rendered
    /// epoch and in-flight flag, and the loop wake. The kernel registers pipes
    /// and reports queue/complete; the handlers here only make requests.
    pub redraw: compositor_support_smithay_state_redraw_schedule::schedule::Schedule,


    // Protocol outboxes — handlers record here (world-free); the rim drains them
    // after dispatch_clients + applies world effects. document/SMITHAY_DECOUPLING.md
    pub committed: Vec<WlSurface>,
    pub new_toplevels: Vec<Window>,
    /// The `bool` is "was this toplevel being carried by a live
    /// `xdg_toplevel_drag_v1` at the moment it was destroyed" — sampled at
    /// destroy time rather than at drain time, since the drag may well have
    /// ended by the time the drain runs.
    pub destroyed_toplevels: Vec<(ToplevelSurface, bool)>,
    pub fullscreen_requests: Vec<(ToplevelSurface, bool)>,
    pub new_layers: Vec<(LayerSurface, Option<WlOutput>, WlrLayer, String)>,
    pub destroyed_layers: Vec<LayerSurface>,
    pub pending_dmabuf: Vec<(DmabufGlobal, Dmabuf, ImportNotifier)>,
    pub geometries: HashMap<WlSurface, Rectangle<i32, Logical>>,
    // Outputs (+ logical geometry) mirrored from the spatial world's Space each
    // drain. Layer-shell popup constrain needs the parent layer's output/size
    // synchronously inside the world-FREE `new_popup` handler (geometry must be set
    // before the initial configure), and that handler can't reach the world Space —
    // so the rim mirrors it here. Cheap: one or two outputs.
    pub outputs_snapshot: Vec<(smithay::output::Output, Rectangle<i32, Logical>)>,
    // True while an xdg-popup explicit grab (a menu) holds the seat. Set by
    // `establish_popup_grab`, cleared by the seat press path when the seat is no longer
    // grabbed. Lets that path drive THIS grab's buttons without touching the DnD grab
    // (which never sets this).
    pub in_popup_grab: bool,
    // Pointer-constraint restoration tokens (seat warp); drain performs them.
    pub pending_restoration: Vec<(WlSurface, Point<f64, Logical>)>,
    // Syncobj fence sources recorded by the pre-commit hook (which has no
    // loop_handle); the rim drain inserts them via `Wire::loop_handle`.
    pub pending_blockers: Vec<(Weak<WlSurface>, DrmSyncPointSource)>,
    // Deferred pointer-constraint activate-on-focus: `focus_changed` records the newly
    // focused surface here (it can't touch the pointer inline — see that handler), and
    // the drain does the is-pointer-over check + `constraint.activate()` with the pointer
    // unlocked. `None` = nothing pending.
    pub pending_constraint_activation: Option<WlSurface>,
}

// ── Redraw scheduling (inlined; handlers call these on Dispatch) ──────────────
impl Dispatch {
    /// Animation continuation: keep an already-running cycle going for the next
    /// frame. Called from inside the scene build by perpetual sources — the
    /// parallax background re-arms this on EVERY composited frame, which is what
    /// makes the compositor free-run without any client involvement.
    ///
    /// That is exactly what exclusive pacing must stop. Gating only client
    /// commits is not enough: the background alone sustains the loop, so the
    /// pacer's cadence could never reach the flip rate. Under exclusive pacing
    /// the tagged client is the sole continuation source, and the floor watchdog
    /// (`pacer::FLOOR`) is what keeps animation alive at 30fps regardless.
    ///
    /// Event-driven redraws (`schedule_redraw`) and the in-flight re-arm
    /// (`rearm_redraw`) are deliberately NOT gated here — those are what let you
    /// pan away from a pacer that has stopped committing.
    #[inline]
    pub fn schedule_redraw_post_vblank(&mut self) {
        if compositor_support_smithay_state_tearing_gate::gate::engaged() {
            return;
        }
        self.redraw.request_silent();
    }
    /// Event-driven redraw (input, window lifecycle, popups, OSK, capture …).
    ///
    /// Under exclusive pacing this is silenced: the tagged client is the ONLY
    /// thing permitted to start a frame, so the flip cadence is exactly its
    /// commit cadence. State changes still land — a camera pan updates the
    /// camera, it simply does not schedule a frame of its own — and the next
    /// pacer-driven (or floor-watchdog) composite renders them.
    ///
    /// This makes `pacer::FLOOR` load-bearing: it is now the only thing keeping
    /// the desktop responsive while paced, and the only reason panning away from
    /// a stalled pacer can release engagement at all (engagement is recomputed
    /// per frame from the visible set, so it needs frames to be produced).
    #[inline]
    pub fn schedule_redraw(&mut self) {
        if compositor_support_smithay_state_tearing_gate::gate::engaged() { return; }
        self.schedule_redraw_unchecked();
    }
    /// The scheduling body, bypassing the exclusive-pacing gate. Used for the
    /// pacer's own commits, which by definition must always be able to drive a
    /// frame — they are the cadence. Every call wakes the loop if any pipe is
    /// idle: a wake is idempotent (pings coalesce) and the executor renders only
    /// pipes behind the epoch, so there is nothing to suppress — and suppressing
    /// it is how paced commits once went unserviced until the floor watchdog.
    #[inline]
    pub fn schedule_redraw_unchecked(&mut self) { self.redraw.request(); }
    /// Unconditional wake: rescues (watchdogs, rate-cap timer) and hotplug, where
    /// an idle cycle must restart whatever the in-flight bookkeeping says.
    #[inline]
    pub fn force_redraw(&mut self) { self.redraw.force(); }
    /// Mark every pipe stale WITHOUT a wake, for callers that invoke the executor
    /// themselves (session resume, the off-thread publish wake).
    #[inline]
    pub fn bump_redraw_epoch(&mut self) { self.redraw.request_silent(); }
}

/// Establish a popup's explicit grab. Called INLINE from `XdgShellHandler::grab` (while the
/// popup is still alive — deferring it let clients tear the popup down first). The one seat
/// operation that used to deadlock from here — `is_pointer_over` via `focus_changed` inside a
/// `pointer.motion` teardown — is deferred separately (`pending_constraint_activation`), so
/// `set_grab`/`set_focus` are safe synchronously. Standard smithay popup-grab wiring: keeps
/// pointer/keyboard focus on the popup chain and dismisses it (popup_done) on an outside
/// interaction. `in_popup_grab` lets the seat press path drive this grab's buttons (see the
/// seat's button handler) without disturbing the DnD grab.
pub fn establish_popup_grab(
    dispatch: &mut Dispatch,
    surface: smithay::wayland::shell::xdg::PopupSurface,
    seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
    serial: smithay::utils::Serial,
) {
    use smithay::desktop::{
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, find_popup_root_surface,
    };
    use smithay::input::Seat;
    use smithay::input::pointer::Focus;

    let Some(seat) = Seat::<Dispatch>::from_resource(&seat) else { return };
    let kind = PopupKind::Xdg(surface);
    let Ok(root) = find_popup_root_surface(&kind) else { return };
    let mut grab = match dispatch.popup.state.grab_popup(root, kind, &seat, serial) {
        Ok(grab) => grab,
        Err(_) => return,
    };
    if let Some(keyboard) = seat.get_keyboard() {
        if keyboard.is_grabbed()
            && !(keyboard.has_grab(serial)
                || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
        {
            grab.ungrab(PopupUngrabStrategy::All);
            return;
        }
        keyboard.set_focus(dispatch, grab.current_grab(), serial);
        keyboard.set_grab(dispatch, PopupKeyboardGrab::new(&grab), serial);
    }
    if let Some(pointer) = seat.get_pointer() {
        if pointer.is_grabbed()
            && !(pointer.has_grab(serial)
                || pointer.has_grab(grab.previous_serial().unwrap_or(serial)))
        {
            grab.ungrab(PopupUngrabStrategy::All);
            return;
        }
        pointer.set_grab(dispatch, PopupPointerGrab::new(&grab), serial, Focus::Keep);
    }
    dispatch.in_popup_grab = true;
    dispatch.schedule_redraw();
}

// ── Pointer-constraint helpers (moved down from `Seat<I>`) ────────────────────
// `PointerConstraintRef::deactivate` takes `&mut D` — it calls
// `PointerConstraintsHandler::remove_constraint`, so the constraint is dropped from
// the handler's own bookkeeping rather than merely being told to stop. A `Seat<I>`
// held INSIDE `D` cannot produce that borrow, so these live on `Dispatch`, which
// owns both halves. The state they read and write is still the seat's
// (`unlock_restoration_location`).
impl Dispatch {
    /// Deactivate any active constraint on `surface`.
    ///
    /// The unlock-restoration token is NOT returned: `deactivate` announces every
    /// deactivation through `PointerConstraintsHandler::remove_constraint`, which is
    /// where the token is harvested and queued. That is the one announcement point
    /// now, so a client destroying its own constraint restores on exactly the same
    /// path as the compositor deactivating one.
    pub fn deactivate_constraint_for(
        &mut self,
        surface: &WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Dispatch>,
    ) {
        with_pointer_constraint(surface, pointer, |c| {
            if let Some(c) = c {
                if c.is_active() {
                    c.deactivate(self, surface, pointer);
                }
            }
        });
    }

    /// Take this surface's pending unlock-restoration token — the position the
    /// pointer must be warped to once it is free — if this surface owns one.
    pub fn take_restoration_for(
        &mut self,
        surface: &WlSurface,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (hint_surface, hint_location) = self.seat.unlock_restoration_location.take()?;
        if &hint_surface == surface {
            return Some((hint_surface, hint_location));
        }
        self.seat.unlock_restoration_location = Some((hint_surface, hint_location));
        None
    }

    /// Focus moved: release the outgoing surface's constraint and arm the incoming
    /// one, but only while it also holds the KEYBOARD focus — a pointer lock on a
    /// window the user is not typing into is not something to re-activate silently.
    pub fn reevaluate_pointer_constraints(
        &mut self,
        pointer: &smithay::input::pointer::PointerHandle<Dispatch>,
        previous: Option<&WlSurface>,
        updated: Option<&WlSurface>,
    ) {
        if let Some(old) = previous {
            self.deactivate_constraint_for(old, pointer);
        }
        if let Some(new_surface) = updated {
            if self.seat.is_keyboard_focused(new_surface) && !self.seat.constraints_suspended {
                with_pointer_constraint(new_surface, pointer, |c| {
                    if let Some(c) = c {
                        if !c.is_active() {
                            c.activate();
                        }
                    }
                });
            }
        }
    }

    /// Drop the constraint under the pointer outright and forget any restoration —
    /// the canvas taking over the pointer, where there is no surface to give it back to.
    pub fn abandon_active_constraint(
        &mut self,
        pointer: &smithay::input::pointer::PointerHandle<Dispatch>,
    ) {
        if let Some(surface) = pointer.current_focus() {
            self.deactivate_constraint_for(&surface, pointer);
        }
        // After the deactivate, so the token `remove_constraint` may have just queued
        // is dropped too: the canvas is not giving the pointer back to a surface.
        self.pending_restoration.clear();
        self.seat.unlock_restoration_location = None;
    }

    /// The hand tool took the pointer: drop the active constraint and refuse to
    /// activate another (a game re-requests its lock the instant it is unlocked)
    /// until [`resume_constraints`](Self::resume_constraints).
    pub fn suspend_constraints(&mut self, pointer: &smithay::input::pointer::PointerHandle<Dispatch>) {
        self.seat.constraints_suspended = true;
        self.abandon_active_constraint(pointer);
    }

    /// The hand tool is off: honour constraints again, and arm the one under the
    /// pointer now if its surface also holds the keyboard — the same terms as a
    /// freshly requested one.
    pub fn resume_constraints(&mut self, pointer: &smithay::input::pointer::PointerHandle<Dispatch>) {
        self.seat.constraints_suspended = false;
        if let Some(surface) = pointer.current_focus() {
            if self.seat.is_keyboard_focused(&surface) {
                with_pointer_constraint(&surface, pointer, |c| {
                    if let Some(c) = c { if !c.is_active() { c.activate(); } }
                });
            }
        }
    }
}

// ── SeatHandler for Dispatch (REQUIRED here: `Seat<Dispatch>` field) ──────────
// Inlined from seat.dispatch / seat.focus. `set_data_device_focus` is deferred
// to wire.base via `clipboard.pending_focus` (it needs DataDeviceHandler).
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
                // Queuing is `remove_constraint`'s job now.
                self.deactivate_constraint_for(&old_focus, &pointer);
            }
            // Activate-on-focus is DEFERRED to the drain. This callback can re-enter from
            // INSIDE `pointer.motion` (a popup-grab teardown restores keyboard focus while
            // motion holds the pointer's mutex); `is_pointer_over` → `pointer.current_focus()`
            // would re-lock that same non-reentrant mutex → deadlock. The drain runs with the
            // pointer unlocked, so the pointer-over check + constraint activate are safe there.
            self.pending_constraint_activation = focused.cloned();
            self.seat.previous_focus = focused.cloned();
        }

        // `set_data_device_focus` needs DataDeviceHandler (downstream) — defer it.
        self.clipboard.pending_focus = Some(client);

        // Follow keyboard focus with tablet-pad focus: `leave` the old client's pad,
        // `enter` the new one, so pad button/ring/strip/dial/mode events are gated to
        // the focused client (external zwp_tablet_pad_v2 has no smithay focus model).
        self.tablet.set_pad_focus(focused.cloned(), SERIAL_COUNTER.next_serial());

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

    use smithay::reexports::wayland_protocols::wp::tearing_control::v1::server::{
        wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
        wp_tearing_control_v1::{self, WpTearingControlV1},
    };
    use compositor_support_smithay_dispatch_wire_tearing::tearing as tc;

    impl GlobalDispatch<WpTearingControlManagerV1, ()> for Dispatch {
        fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<WpTearingControlManagerV1>, _: &(), di: &mut DataInit<'_, Self>) {
            di.init(resource, ());
        }
    }
    impl WLDispatch<WpTearingControlManagerV1, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpTearingControlManagerV1, request: wp_tearing_control_manager_v1::Request, _: &(), _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            tc::dispatch_manager(request, di);
        }
    }
    impl WLDispatch<WpTearingControlV1, WlSurface> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &WpTearingControlV1, request: wp_tearing_control_v1::Request, surface: &WlSurface, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
            tc::dispatch_control(request, surface);
        }
    }

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

// ── external zwp_tablet_manager_v2 (tool + pad) ────────────────────────────────
// Hand-rolled like `color_impls`: smithay implements only the tool protocol AND
// hides its seat instances (`pub(crate)`), so we can't advertise pads through it.
// We own the whole stack. Userdata is `()` (state lives in `Dispatch.tablet`),
// cleaned up in `destroyed`. Coexists with `delegate_dispatch2!` because our `()`
// userdata never matches smithay's tablet `Dispatch2` impls (GlobalData/…UserData).
mod tablet_impls {
    use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::{
        zwp_tablet_manager_v2::{self, ZwpTabletManagerV2},
        zwp_tablet_pad_dial_v2::{self, ZwpTabletPadDialV2},
        zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
        zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
        zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
        zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
        zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
        zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
        zwp_tablet_v2::{self, ZwpTabletV2},
    };
    use smithay::reexports::wayland_server::{
        backend::ClientId, Client, DataInit, Dispatch as WLDispatch, DisplayHandle, GlobalDispatch,
        New, Resource,
    };
    use super::Dispatch;

    impl GlobalDispatch<ZwpTabletManagerV2, ()> for Dispatch {
        fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ZwpTabletManagerV2>, _: &(), di: &mut DataInit<'_, Self>) {
            di.init(resource, ());
        }
    }
    impl WLDispatch<ZwpTabletManagerV2, ()> for Dispatch {
        fn request(state: &mut Self, client: &Client, _: &ZwpTabletManagerV2, request: zwp_tablet_manager_v2::Request, _: &(), dh: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            if let zwp_tablet_manager_v2::Request::GetTabletSeat { tablet_seat, .. } = request {
                // y5 is single-seat: track every bound tablet-seat flatly.
                let seat = di.init(tablet_seat, ());
                state.tablet.add_seat::<Dispatch>(dh, &seat, client);
            }
        }
    }
    impl WLDispatch<ZwpTabletSeatV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletSeatV2, _: zwp_tablet_seat_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        fn destroyed(state: &mut Self, _: ClientId, seat: &ZwpTabletSeatV2, _: &()) {
            state.tablet.remove_seat(&seat.id());
        }
    }
    impl WLDispatch<ZwpTabletV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletV2, _: zwp_tablet_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        fn destroyed(state: &mut Self, _: ClientId, tablet: &ZwpTabletV2, _: &()) {
            // NB: `ZwpTabletV2::id` is also a protocol event method — disambiguate.
            state.tablet.remove_tablet_resource(&Resource::id(tablet));
        }
    }
    impl WLDispatch<ZwpTabletToolV2, ()> for Dispatch {
        // Honor `set_cursor`: install the client's tool cursor surface (+ hotspot) as
        // the pointer image, exactly like `wl_pointer::set_cursor` (the cursor follows
        // the pen). `force_cursor` (e.g. the canvas Hand grab) still overrides it at
        // render time; the pen session reverts it on proximity-out.
        fn request(state: &mut Self, _: &Client, tool: &ZwpTabletToolV2, request: zwp_tablet_tool_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
            if let zwp_tablet_tool_v2::Request::SetCursor { surface, hotspot_x, hotspot_y, .. } = request {
                if let Some(status) = state.tablet.set_tool_cursor(tool, surface, (hotspot_x, hotspot_y).into()) {
                    state.seat.pointer_status = status;
                    state.schedule_redraw();
                }
            }
        }
        fn destroyed(state: &mut Self, _: ClientId, tool: &ZwpTabletToolV2, _: &()) {
            state.tablet.remove_tool_resource(&tool.id());
        }
    }
    // ── pad objects (server-created via seat.pad_added; no globals) ──────────────
    impl WLDispatch<ZwpTabletPadV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletPadV2, _: zwp_tablet_pad_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        fn destroyed(state: &mut Self, _: ClientId, pad: &ZwpTabletPadV2, _: &()) {
            state.tablet.remove_pad_resource(&pad.id());
        }
    }
    impl WLDispatch<ZwpTabletPadGroupV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletPadGroupV2, _: zwp_tablet_pad_group_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
    }
    impl WLDispatch<ZwpTabletPadRingV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletPadRingV2, _: zwp_tablet_pad_ring_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        fn destroyed(state: &mut Self, _: ClientId, ring: &ZwpTabletPadRingV2, _: &()) {
            state.tablet.remove_pad_resource(&ring.id());
        }
    }
    impl WLDispatch<ZwpTabletPadStripV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletPadStripV2, _: zwp_tablet_pad_strip_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        fn destroyed(state: &mut Self, _: ClientId, strip: &ZwpTabletPadStripV2, _: &()) {
            state.tablet.remove_pad_resource(&strip.id());
        }
    }
    impl WLDispatch<ZwpTabletPadDialV2, ()> for Dispatch {
        fn request(_: &mut Self, _: &Client, _: &ZwpTabletPadDialV2, _: zwp_tablet_pad_dial_v2::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
        fn destroyed(state: &mut Self, _: ClientId, dial: &ZwpTabletPadDialV2, _: &()) {
            state.tablet.remove_pad_resource(&dial.id());
        }
    }
}

// ── xdg_session_management_v1 impls (orphan-required here) ─────────────────────
// Hand-rolled like `color_impls` / `tablet_impls`: smithay has no module for the
// protocol and `wayland-protocols` ships the staging XML without bindings, so
// `wire.session` scans it and owns the request logic; the store lives on
// `Dispatch.session`.
mod session_impls {
    use compositor_support_smithay_dispatch_wire_session::session::{
        self, SessionData, SessionResource, ToplevelSessionData, XdgSessionManagerV1, XdgSessionV1,
        XdgToplevelSessionV1, XxSessionManagerV1, XxSessionV1, XxToplevelSessionV1,
        xdg_session_manager_v1, xdg_session_v1, xdg_toplevel_session_v1, xx_session_manager_v1,
        xx_session_v1, xx_toplevel_session_v1,
    };
    use smithay::reexports::wayland_server::{
        backend::ClientId, Client, DataInit, Dispatch as WLDispatch, DisplayHandle, GlobalDispatch,
        New,
    };
    use super::Dispatch;

    impl GlobalDispatch<XdgSessionManagerV1, ()> for Dispatch {
        fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<XdgSessionManagerV1>, _: &(), di: &mut DataInit<'_, Self>) {
            di.init(resource, ());
        }
    }
    impl WLDispatch<XdgSessionManagerV1, ()> for Dispatch {
        fn request(state: &mut Self, client: &Client, manager: &XdgSessionManagerV1, request: xdg_session_manager_v1::Request, _: &(), dh: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            session::dispatch_manager(&mut state.session, &mut state.session_live, manager, client, dh, request, di);
        }
    }
    impl WLDispatch<XdgSessionV1, SessionData> for Dispatch {
        fn request(state: &mut Self, _: &Client, session: &XdgSessionV1, request: xdg_session_v1::Request, data: &SessionData, _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            session::dispatch_session(&mut state.session, session, &data.session_id, request, di);
        }
        // Release the live claim; the REMEMBERED names stay, which is what makes
        // the next run restorable. Only an explicit `remove` erases those.
        fn destroyed(state: &mut Self, _: ClientId, resource: &XdgSessionV1, data: &SessionData) {
            session::destroyed_session(&mut state.session_live, data, SessionResource::Xdg(resource.clone()));
        }
    }
    impl WLDispatch<XdgToplevelSessionV1, ToplevelSessionData> for Dispatch {
        fn request(state: &mut Self, _: &Client, _: &XdgToplevelSessionV1, request: xdg_toplevel_session_v1::Request, data: &ToplevelSessionData, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
            if let Some(renamed) =
                session::dispatch_toplevel_session(&mut state.session, data, request)
            {
                state.session_live.renames.push(renamed);
            }
        }
        fn destroyed(state: &mut Self, _: ClientId, _: &XdgToplevelSessionV1, data: &ToplevelSessionData) {
            session::destroyed_toplevel_session(&mut state.session, data);
        }
    }

    // The same protocol under its pre-rename `xx_` namespace — what GTK 4.22,
    // Qt 6.11 and Chrome 151 bind. Separate impls because the wire shapes
    // genuinely differ (see `wire.session::legacy`); the store behind them is
    // the same one.
    impl GlobalDispatch<XxSessionManagerV1, ()> for Dispatch {
        fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<XxSessionManagerV1>, _: &(), di: &mut DataInit<'_, Self>) {
            di.init(resource, ());
        }
    }
    impl WLDispatch<XxSessionManagerV1, ()> for Dispatch {
        fn request(state: &mut Self, client: &Client, manager: &XxSessionManagerV1, request: xx_session_manager_v1::Request, _: &(), dh: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            session::dispatch_legacy_manager(&mut state.session, &mut state.session_live, manager, client, dh, request, di);
        }
    }
    impl WLDispatch<XxSessionV1, SessionData> for Dispatch {
        fn request(state: &mut Self, _: &Client, session: &XxSessionV1, request: xx_session_v1::Request, data: &SessionData, _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            session::dispatch_legacy_session(&mut state.session, session, &data.session_id, request, di);
        }
        fn destroyed(state: &mut Self, _: ClientId, resource: &XxSessionV1, data: &SessionData) {
            session::destroyed_session(&mut state.session_live, data, SessionResource::Xx(resource.clone()));
        }
    }
    impl WLDispatch<XxToplevelSessionV1, ToplevelSessionData> for Dispatch {
        fn request(state: &mut Self, _: &Client, _: &XxToplevelSessionV1, request: xx_toplevel_session_v1::Request, data: &ToplevelSessionData, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
            session::dispatch_legacy_toplevel_session(&mut state.session, data, request);
        }
        fn destroyed(state: &mut Self, _: ClientId, _: &XxToplevelSessionV1, data: &ToplevelSessionData) {
            session::destroyed_toplevel_session(&mut state.session, data);
        }
    }
}

// ── xdg_toplevel_drag_v1 impls (orphan-required here) ─────────────────────────
// Unlike the session protocol, `wayland-protocols` DOES generate bindings for
// this one (staging feature, already on via smithay), so `wire.drag` owns only
// the request logic; the source→drag registry lives on `Dispatch.toplevel_drag`.
mod drag_impls {
    use compositor_support_smithay_dispatch_wire_drag::drag::{
        self, ToplevelDragData, XdgToplevelDragManagerV1, XdgToplevelDragV1,
        xdg_toplevel_drag_manager_v1, xdg_toplevel_drag_v1,
    };
    use smithay::reexports::wayland_server::{
        Client, DataInit, Dispatch as WLDispatch, DisplayHandle, GlobalDispatch, New,
    };
    use super::Dispatch;

    impl GlobalDispatch<XdgToplevelDragManagerV1, ()> for Dispatch {
        fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<XdgToplevelDragManagerV1>, _: &(), di: &mut DataInit<'_, Self>) {
            di.init(resource, ());
        }
    }
    impl WLDispatch<XdgToplevelDragManagerV1, ()> for Dispatch {
        fn request(state: &mut Self, _: &Client, manager: &XdgToplevelDragManagerV1, request: xdg_toplevel_drag_manager_v1::Request, _: &(), _: &DisplayHandle, di: &mut DataInit<'_, Self>) {
            drag::dispatch_manager(&mut state.toplevel_drag, manager, request, di);
        }
    }
    impl WLDispatch<XdgToplevelDragV1, ToplevelDragData> for Dispatch {
        fn request(state: &mut Self, _: &Client, resource: &XdgToplevelDragV1, request: xdg_toplevel_drag_v1::Request, data: &ToplevelDragData, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
            drag::dispatch_drag(&mut state.toplevel_drag, resource, request, data);
        }
    }
}

// ── wlr-foreign-toplevel-management impls (orphan-required here) ────────────────
// smithay has no wlr foreign-toplevel handler, so the manager + handle
// GlobalDispatch/Dispatch impls are hand-written against the raw wlr bindings and
// delegate to `foreign.base` (state + emit). Inbound control requests are queued
// world-free onto `self.foreign`; the rim drains + applies them.
mod foreign_impls {
    use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
        zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
        zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
    };
    use smithay::reexports::wayland_server::{
        Client, DataInit, Dispatch as WLDispatch, DisplayHandle, GlobalDispatch, New,
        backend::ClientId,
    };
    use smithay::wayland::foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState};
    use compositor_support_smithay_state_foreign_base::base::{
        ForeignManagerGlobalData, ForeignRequest, ToplevelHandleData,
    };
    use super::Dispatch;

    // ext_foreign_toplevel_list_v1: smithay drives the protocol (Dispatch/GlobalDispatch
    // come from `delegate_dispatch2!(Dispatch)`); we only supply the state accessor.
    impl ForeignToplevelListHandler for Dispatch {
        fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
            self.foreign.ext_state()
        }
    }

    impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignManagerGlobalData> for Dispatch {
        fn bind(state: &mut Self, _dh: &DisplayHandle, _client: &Client, resource: New<ZwlrForeignToplevelManagerV1>, _data: &ForeignManagerGlobalData, di: &mut DataInit<'_, Self>) {
            let manager = di.init(resource, ());
            state.foreign.bind_manager::<Dispatch>(manager);
        }
    }

    impl WLDispatch<ZwlrForeignToplevelManagerV1, ()> for Dispatch {
        fn request(state: &mut Self, _client: &Client, manager: &ZwlrForeignToplevelManagerV1, request: zwlr_foreign_toplevel_manager_v1::Request, _data: &(), _dh: &DisplayHandle, _di: &mut DataInit<'_, Self>) {
            match request {
                zwlr_foreign_toplevel_manager_v1::Request::Stop => {
                    state.foreign.remove_manager(manager);
                    manager.finished();
                }
                _ => {}
            }
        }
        fn destroyed(state: &mut Self, _client: ClientId, manager: &ZwlrForeignToplevelManagerV1, _data: &()) {
            state.foreign.remove_manager(manager);
        }
    }

    impl WLDispatch<ZwlrForeignToplevelHandleV1, ToplevelHandleData> for Dispatch {
        fn request(state: &mut Self, _client: &Client, _handle: &ZwlrForeignToplevelHandleV1, request: zwlr_foreign_toplevel_handle_v1::Request, data: &ToplevelHandleData, _dh: &DisplayHandle, _di: &mut DataInit<'_, Self>) {
            use zwlr_foreign_toplevel_handle_v1::Request as R;
            let surface = data.surface.clone();
            match request {
                // Supported set only. maximize/minimize are intentionally NOT handled — y5
                // has no such window model, so they fall through to the ignore arm rather than
                // manufacturing a request the rim discards (docks then don't act on them).
                R::Activate { .. } => state.foreign.push_request(surface, ForeignRequest::Activate),
                R::Close => state.foreign.push_request(surface, ForeignRequest::Close),
                R::SetFullscreen { .. } => state.foreign.push_request(surface, ForeignRequest::Fullscreen(true)),
                R::UnsetFullscreen => state.foreign.push_request(surface, ForeignRequest::Fullscreen(false)),
                // set_maximized/unset_maximized, set_minimized/unset_minimized, set_rectangle,
                // destroy: accepted by the protocol, ignored by us.
                _ => {}
            }
        }
        fn destroyed(state: &mut Self, _client: ClientId, handle: &ZwlrForeignToplevelHandleV1, _data: &ToplevelHandleData) {
            state.foreign.remove_handle(handle);
        }
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
    use smithay::reexports::wayland_server::backend::ClientId;
    use smithay::backend::allocator::dmabuf::Dmabuf;
    use smithay::backend::renderer::utils::on_commit_buffer_handler;
    use smithay::desktop::{PopupKind, PopupManager, WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output};
    use smithay::input::{Seat, SeatState};
    use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
    use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
    use compositor_support_smithay_dispatch_wire_drag::drag::{ToplevelDragData, ToplevelDragHost};
    use compositor_support_smithay_state_grab_drag_state::drag_state::ToplevelDragGrab;
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
    use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
    use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler};
    use compositor_support_smithay_state_clipboard_policy::policy;
    use compositor_support_smithay_state_clipboard_pump::pump;
    use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState};
    use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState};
    use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
    use smithay::wayland::shell::xdg::dialog::{ToplevelDialogHint, XdgDialogHandler};
    use smithay::wayland::shm::{ShmHandler, ShmState};
    use smithay::input::tablet::TabletSeatHandler;
    use smithay::wayland::xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData};
    use smithay::wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState};
    use smithay::wayland::xdg_toplevel_icon::XdgToplevelIconHandler;
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
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

    /// Constrain a layer-shell popup to the OUTPUT its parent layer surface sits on,
    /// so a menu off a bar flips/slides to stay on-screen instead of overflowing.
    /// (Window popups stay unconstrained via `unconstrain_popup`; IME popups are
    /// positioned by smithay against the text cursor — both intentionally unchanged.)
    ///
    /// The geometry MUST be set before the popup's initial configure, which the commit
    /// handler sends on the popup's first commit — same dispatch as `get_popup` for GTK.
    /// So this runs synchronously in the handler, not the drain, and reads the outputs
    /// from `outputs_snapshot` (the world-free handler can't reach the world's Space).
    fn constrain_layer_popup(dispatch: &Dispatch, parent: &LayerSurface, popup: &PopupSurface) {
        for (output, output_geo) in &dispatch.outputs_snapshot {
            let map = layer_map_for_output(output);
            let Some(layer) = map.layer_for_surface(parent.wl_surface(), WindowSurfaceType::TOPLEVEL)
            else {
                continue;
            };
            let Some(layer_geo) = map.layer_geometry(layer) else {
                continue;
            };
            // Target = the output rect (output-local, so origin 0) expressed in the
            // popup's parent-relative space: shift by −the layer's position on the
            // output and −the offset from the layer down to this popup's immediate
            // parent (non-zero only for nested submenus).
            let offset = get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
            let target = Rectangle::from_loc_and_size(
                Point::from((0, 0)) - layer_geo.loc - offset,
                output_geo.size,
            );
            popup.with_pending_state(|state| {
                state.geometry = state.positioner.get_unconstrained_geometry(target);
            });
            return;
        }
    }

    impl PointerConstraintsHandler for Dispatch {
        fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
            let pointer_focused = pointer.current_focus().map(|f| &f == surface).unwrap_or(false);
            if !pointer_focused { return; }
            if !self.seat.is_keyboard_focused(surface) { return; }
            // Hand tool: the request stays registered but inactive; `resume_constraints` arms it.
            if self.seat.constraints_suspended { return; }
            with_pointer_constraint(surface, pointer, |c| { if let Some(c) = c { if !c.is_active() { c.activate(); } } });
        }
        /// The ONE place a constraint going away is announced — a client destroying
        /// it, or `PointerConstraintRef::deactivate` from our own paths. Harvest the
        /// unlock-restoration hint here so both routes warp the pointer identically.
        /// Queued rather than applied: this can run from inside `pointer.motion`,
        /// where warping would re-enter the pointer's own (non-reentrant) mutex.
        fn remove_constraint(
            &mut self,
            surface: &WlSurface,
            _pointer: &PointerHandle<Self>,
            _constraint: Option<&smithay::wayland::pointer_constraints::PointerConstraint>,
        ) {
            if let Some(token) = self.take_restoration_for(surface) {
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
    /// Upstream's tablet infrastructure now carries a tool-focus target type. y5's
    /// tablet support is hand-rolled (`wire.tablet`) and routes tool events to the
    /// focused surface directly, so the plain `WlSurface` — the same focus type the
    /// seat uses — is the target here.
    impl TabletSeatHandler for Dispatch {
        type ToolFocus = WlSurface;
    }
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
    impl XdgDialogHandler for Dispatch {
        /// Latch modality onto the surface while it is still true.
        ///
        /// `xdg_dialog_v1`'s destructor resets the hint back to `Unknown`, and a
        /// well-behaved client fires it alongside the toplevel — so a reader at
        /// teardown would see nothing. The mark is sticky for that reason.
        ///
        /// Only `Modal` marks. A plain `Dialog` is a client saying "I am
        /// subordinate", which floating find/replace panels and tool palettes also
        /// say; it is logged so the decision to widen can be made on evidence.
        fn dialog_hint_changed(&mut self, toplevel: ToplevelSurface, hint: ToplevelDialogHint) {
            match hint {
                ToplevelDialogHint::Modal => {
                    compositor_support_smithay_state_ephemeral_mark::mark::mark(toplevel.wl_surface());
                    trace!("xdg-dialog: toplevel marked modal");
                }
                ToplevelDialogHint::Dialog => trace!("xdg-dialog: toplevel marked dialog (non-modal)"),
                ToplevelDialogHint::Unknown => trace!("xdg-dialog: toplevel hint cleared"),
            }
        }
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
            // A dead toplevel holds no session name (see `SessionStore::release_toplevel`).
            self.session.release_toplevel(surface.xdg_toplevel().id().protocol_id());
            // Carried right now, or abandoned by a cancel of ours. Consumed on the
            // way past: the verdict belongs to this one destroy.
            let abandoned = self
                .toplevel_drag
                .abandoned
                .iter()
                .position(|s| s == surface.wl_surface());
            if let Some(at) = abandoned {
                self.toplevel_drag.abandoned.remove(at);
            }
            let carried = abandoned.is_some()
                || self.toplevel_drag.is_carrying(surface.wl_surface());
            self.destroyed_toplevels.push((surface, carried));
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
        fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
            // Honor the client's explicit popup grab (menus / GTK popovers — incl. old GTK
            // apps like GIMP that DISMISS the popup if the grab isn't honored). This is the
            // ONLY grab we accept; window move/resize grabs stay disabled by design.
            //
            // Establish it INLINE, while the popup is still alive. Deferring to the drain let
            // the client tear the popup down first (we observed alive=false at drain → the grab
            // silently no-op'd → GTK re-tried → the "click twice" bug). The re-entrant deadlock
            // that once motivated deferral was the `is_pointer_over` call in `focus_changed`
            // (only reachable via a grab teardown inside `pointer.motion`); that is now deferred
            // on its own (`pending_constraint_activation`), so the seat grab is safe here.
            super::establish_popup_grab(self, surface, seat, serial);
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
    // ── Clipboard persistence ────────────────────────────────────────────────
    // A wayland selection is a live `wl_data_source` owned by the copying client, and
    // there is no OS-level clipboard behind it — so when the app exits, the clipboard
    // is gone. These three hooks snapshot the bytes while the source is alive and hand
    // them back at the moment it dies. `clipboard.capture` holds the slot; the reads
    // and writes themselves are pumped by `wire.clipboard` on the event loop.
    //
    // Nothing here may log clipboard CONTENT — mime names, byte counts and generations
    // only.
    impl SelectionHandler for Dispatch {
        // The capture generation the selection was installed against, so a read that
        // raced a newer copy is refused instead of served stale.
        type SelectionUserData = u64;

        fn new_selection(
            &mut self,
            ty: SelectionTarget,
            source: Option<SelectionSource>,
            _seat: Seat<Self>,
        ) {
            if ty != SelectionTarget::Clipboard {
                return;
            }
            // Invalidate FIRST: bump the generation and drop the head before a single
            // byte of the new selection is read. Without this an in-flight capture that
            // completes late would refill the slot with the copy the user just replaced.
            let advertised = source.as_ref().map(|s| s.mime_types().len()).unwrap_or(0);
            let generation = self.clipboard.capture.arm(advertised);
            // Cancel whatever the worker is still reading, including when the clipboard was
            // merely CLEARED — that path returns below without ever reaching the drain, so
            // this is the only thing that stops the superseded reads.
            if let Some(worker) = self.clipboard.worker.as_ref() {
                worker.arm(generation);
            }
            let Some(source) = source else {
                trace!("clipboard cleared by client generation={generation}");
                return;
            };
            let mime_types = policy::ordered(&source.mime_types());
            trace!(
                "clipboard copy generation={generation} flavors={advertised} \
                 mimes={mime_types:?}"
            );
            // Deferred, NOT captured here: smithay calls us before it installs the new
            // selection, so reading now would read the previous clipboard.
            self.clipboard.pending_capture = Some((generation, mime_types));
        }

        fn selection_source_destroyed(
            &mut self,
            ty: SelectionTarget,
            _seat: &Seat<Self>,
        ) -> Option<(Vec<String>, Self::SelectionUserData)> {
            if ty != SelectionTarget::Clipboard {
                return None;
            }
            // The owning client is gone and smithay is about to clear the clipboard.
            // Answering here rather than re-installing afterwards is what keeps this to
            // ONE state transition: clients never see `selection(nil)` followed by a
            // fresh offer.
            //
            // Reads still in flight are taken back from the worker and finished in place.
            // Safe without blocking: the client's exit closed every write end (that is why
            // we are in its destructor) and our own copy was dropped at arm time, so the
            // pipes have no writer left and drain straight to EOF. This is what rescues the
            // common copy-then-immediately-close, where a transfer is still moving at exit.
            //
            // No redraw is scheduled: this runs inside `dispatch_clients`, and the
            // clipboard has no on-screen representation of its own.
            let generation = self.clipboard.capture.generation();
            // First, whatever the worker finished but the drain has not picked up yet.
            let ready = self
                .clipboard
                .worker
                .as_ref()
                .map(|worker| worker.collect())
                .unwrap_or_default();
            for done in ready {
                self.clipboard.capture.admit(
                    done.generation,
                    done.order,
                    done.mime,
                    done.bytes,
                    policy::BUDGET,
                );
            }
            let mut readers = self
                .clipboard
                .worker
                .as_ref()
                .map(|worker| worker.reclaim())
                .unwrap_or_default();
            for reader in readers.iter_mut() {
                // What the budget has left, not the whole budget: flavors are finished
                // one at a time here, and each is admitted before the next is read, so
                // `used` is the running total. Passing the full budget instead would let
                // every remaining flavor buffer all of it before `admit` refused it.
                let headroom = self.clipboard.capture.headroom(policy::BUDGET);
                if !matches!(pump::pump(reader, headroom), pump::Pump::Eof) {
                    // Blocked means somebody else still holds the write end (a forked
                    // child); nothing more will arrive for us. Don't spin — drop it.
                    continue;
                }
                let bytes = std::mem::take(&mut reader.buffer);
                self.clipboard.capture.admit(
                    reader.generation,
                    reader.order,
                    reader.mime.clone(),
                    bytes,
                    policy::BUDGET,
                );
            }

            if self.clipboard.capture.is_empty() {
                trace!("clipboard owner gone with nothing captured generation={generation}");
                return None;
            }
            let mime_types = self.clipboard.capture.mime_types();
            // captured < advertised means the offer shrank across the exit — the one
            // thing a receiver can observe that the user never asked for.
            info!(
                "clipboard persisted across client exit generation={generation} \
                 captured={} advertised={} bytes={} mimes={mime_types:?}",
                mime_types.len(),
                self.clipboard.capture.advertised(),
                self.clipboard.capture.used()
            );
            Some((mime_types, generation))
        }

        fn send_selection(
            &mut self,
            ty: SelectionTarget,
            mime_type: String,
            fd: std::os::fd::OwnedFd,
            _seat: Seat<Self>,
            user_data: &Self::SelectionUserData,
            client: ClientId,
        ) {
            if ty != SelectionTarget::Clipboard {
                return;
            }
            // Async: a payload past the pipe buffer would block the compositor if it
            // were written inline. Dropping `fd` on a mismatch closes the pipe, which
            // the pasting client reads as an empty transfer.
            if !self.clipboard.capture.is_current(*user_data) {
                trace!("clipboard read refused, stale generation={user_data}");
                return;
            }
            // No redraw: `receive` is handled inside `dispatch_clients`, so `serve` arms
            // the writer in the same iteration's drain, and what the user actually sees
            // is the PASTING client committing a buffer — which schedules its own redraw
            // through the normal commit path.
            self.clipboard.pending_sends.push((*user_data, client, mime_type, fd));
        }
    }
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

    /// The surface's commit counter as the pacer last saw it, in its `UserDataMap`:
    /// a gated commit is admitted only when this advanced (new pixels).
    struct LastPacedCommit(std::sync::Mutex<Option<smithay::backend::renderer::utils::CommitCounter>>);

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
            // Exclusive pacing: while a tagged client is visible, only tagged
            // clients' commits may drive the flip cadence — that is what makes
            // every flip carry exactly one of their frames. The commit itself is
            // still fully processed above; only the redraw trigger is withheld,
            // so a neighbour's content is simply picked up by the next composite
            // a pacer causes.
            //
            // Scoped to CLIENT commits on purpose. Input, camera and animation
            // redraws all reach `schedule_redraw` by other paths and stay live,
            // which is what lets you pan away from a pacer that has stopped
            // committing instead of being stuck looking at it.
            {
                use compositor_support_smithay_state_tearing_gate::gate;
                use compositor_support_smithay_state_tearing_pacer::pacer;
                let g = gate::get();
                if g == gate::Gate::Off {
                    self.schedule_redraw();
                } else {
                    // Each gate asks a different question of the surface, so the
                    // properties are resolved here rather than assuming the tag:
                    // `Focused` admits the focused window whether or not it is
                    // tagged, and `Visible` admits anything the scene drew.
                    //
                    // The gate's properties belong to the WINDOW, which is the
                    // tree's root surface: keyboard focus is held by the root, and
                    // the scene stamps the root as visible. The commit, though, may
                    // arrive on a SUBSURFACE — native games commonly present their
                    // swapchain on one under the xdg toplevel, and smithay invokes
                    // this handler once per surface of a transaction — so a
                    // subsurface's frame must be judged by its root, or the game
                    // never passes the gate and the loop runs at the floor.
                    let root = {
                        let mut r = surface.clone();
                        while let Some(p) = compositor::get_parent(&r) {
                            r = p;
                        }
                        r
                    };
                    // Focus is resolved ONLY for the gates that read it. It costs
                    // a seat lookup plus a focus-target clone, and this runs on
                    // every client commit — thousands a second under tearing —
                    // while `Tagged` and `Visible` never look at it.
                    let focused = matches!(g, gate::Gate::TaggedFocused | gate::Gate::Focused)
                        && self
                            .seat
                            .seat
                            .get_keyboard()
                            .and_then(|kb| kb.current_focus())
                            .is_some_and(|f| f == root);
                    let frame = gate::frame();
                    let visible = compositor::with_states(&root, |states| {
                        states
                            .data_map
                            .get::<gate::VisibleSurface>()
                            .is_some_and(|v| v.fresh(frame))
                    });
                    // Window-level, like the scene's engagement test: a tag
                    // anywhere in the tree makes every pixel-carrying commit of
                    // that window a frame. Otherwise a game's HUD or overlay
                    // subsurface would engage the gate yet never tick it.
                    let tagged = {
                        use smithay::wayland::compositor::{with_surface_tree_downward, TraversalAction};
                        let mut found = false;
                        with_surface_tree_downward(
                            &root,
                            (),
                            |_, _, _| TraversalAction::DoChildren(()),
                            |_, states, _| {
                                found |= states
                                    .data_map
                                    .get::<pacer::PacerSurface>()
                                    .is_some_and(|t| t.get());
                            },
                            |_, _, _| true,
                        );
                        found
                    };
                    // Only a commit that carries NEW PIXELS is a frame. Smithay's
                    // per-surface commit counter advances exactly when a commit
                    // attaches a new buffer with damage; a commit that attaches
                    // none — a frame-callback request, an input-region or
                    // cursor-hint update — leaves it where it was. Xwayland (via
                    // satellite) commits a game's toplevel on every pointer event
                    // that way, ~1000/s under a wired mouse, and each one used to
                    // tick the pacer: composites above the client's frame rate,
                    // every one a tear spent on nothing new.
                    let advanced = {
                        use smithay::backend::renderer::utils::with_renderer_surface_state;
                        let now = with_renderer_surface_state(surface, |s| s.current_commit());
                        compositor::with_states(surface, |states| {
                            states.data_map.insert_if_missing(|| LastPacedCommit(std::sync::Mutex::new(None)));
                            let slot = states.data_map.get::<LastPacedCommit>().unwrap();
                            let mut last = slot.0.lock().unwrap_or_else(|e| e.into_inner());
                            let advanced = now.is_some() && now != *last;
                            *last = now;
                            advanced
                        })
                    };
                    // `_unchecked` because `schedule_redraw` is itself gated while
                    // engaged — this IS the cadence, so it must bypass the gate.
                    if advanced && g.admits(tagged, focused, visible) {
                        self.schedule_redraw_unchecked();
                    }
                }
            }
        }
    }
    impl DmabufHandler for Dispatch {
        fn dmabuf_state(&mut self) -> &mut DmabufState { &mut self.dmabuf.state }
        fn dmabuf_imported(&mut self, global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
            self.pending_dmabuf.push((global.clone(), dmabuf, notifier));
        }
    }
    impl ToplevelDragHost for Dispatch {
        fn queue_toplevel_drag_move(&mut self, surface: WlSurface, location: Point<f64, Logical>) {
            // Last write wins: the grab emits one of these per motion event, and
            // only the newest position is meaningful by the time the drain runs.
            if let Some(slot) = self.toplevel_drag.moves.iter_mut().find(|(s, _)| *s == surface) {
                slot.1 = location;
            } else {
                self.toplevel_drag.moves.push((surface, location));
            }
            self.schedule_redraw();
        }
    }
    impl DndGrabHandler for Dispatch {
        fn cancelled(&mut self, _: Seat<Self>, _: Point<f64, Logical>) {
            self.dnd.icon = None;
            self.toplevel_drag.settled.extend(self.toplevel_drag.carried_surface());
            // Anything the grab queued on its last motion is now a position for a
            // drag that is over. The frame hook applies these, so leaving them
            // would move the window once more after the carry ended.
            self.toplevel_drag.moves.clear();
            self.toplevel_drag.deactivate_all();
            self.schedule_redraw();
        }
        fn dropped(&mut self, _: Option<smithay::input::dnd::DndTarget<'_, Self>>, _: bool, _: Seat<Self>, _: Point<f64, Logical>) {
            self.dnd.icon = None;
            // The dragged toplevel keeps the position it was carried to: the
            // spec settles it "as if a xdg_toplevel_move operation ended".
            // Read the carried surface BEFORE deactivating — that is what
            // `carried_surface` filters on.
            //
            // This runs with the drag still active: on a physical drop the inner
            // `DnDGrab` is what gets unset (it passes ITSELF to `unset_grab`), so
            // the wrapper's `unset` — which clears `active` — only runs afterwards.
            self.toplevel_drag.settled.extend(self.toplevel_drag.carried_surface());
            self.toplevel_drag.deactivate_all();
            self.schedule_redraw();
        }
    }
    impl WaylandDndGrabHandler for Dispatch {
        fn dnd_requested<Src: Source>(&mut self, source: Src, icon: Option<WlSurface>, seat: Seat<Dispatch>, serial: Serial, type_: GrabType) {
            match type_ {
                GrabType::Pointer => {
                    let Some(ptr) = seat.get_pointer() else { return; };
                    let Some(start_data) = ptr.grab_start_data() else { return; };
                    // `Src` is concretely `WlDataSource` on the wayland path
                    // (`data_device::device`), which is how a toplevel drag
                    // created for this source is recovered. Downcast rather
                    // than widen the handler: other Source impls (xwayland,
                    // client-local) have no drag object and must stay plain.
                    let attached = (&source as &dyn std::any::Any)
                        .downcast_ref::<WlDataSource>()
                        .and_then(|s| self.toplevel_drag.for_source(s).cloned());
                    // Where the drag started, captured before `start_data` is
                    // consumed by the grab. A drag that goes on to attach THIS
                    // toplevel has torn nothing off — see `ToplevelDragData::live`.
                    let origin = start_data.focus.as_ref().map(|(s, _)| s.clone());
                    self.dnd.icon = icon;
                    let grab = DnDGrab::new_pointer(&self.output.display_handle, start_data, source, seat);
                    match attached {
                        Some(drag) => {
                            if let Some(d) = drag.data::<ToplevelDragData>() {
                                d.set_active(true);
                                d.set_origin(origin.as_ref());
                            }
                            ptr.set_grab(self, ToplevelDragGrab::new(grab, drag), serial, Focus::Keep);
                        }
                        None => ptr.set_grab(self, grab, serial, Focus::Keep),
                    }
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
        // A popup parented to a layer surface (menu off a panel/bar). smithay routes
        // these here rather than through `XdgShellHandler::new_popup` (the popup is
        // created with a NULL xdg parent, then adopted by the layer). Track it in the
        // shared PopupManager so the commit handler sends its initial configure and the
        // draw + hit paths (which walk `PopupManager::popups_for_surface`) find it.
        fn new_popup(&mut self, parent: LayerSurface, popup: smithay::wayland::shell::xdg::PopupSurface) {
            // Layer popups are constrained to their output (unlike window popups).
            constrain_layer_popup(self, &parent, &popup);
            let _ = self.popup.state.track_popup(PopupKind::Xdg(popup));
            self.schedule_redraw();
        }
    }
    impl OutputHandler for Dispatch {}
    /// `xdg_toplevel_icon_v1`. The icon itself is double-buffered on the surface
    /// (`ToplevelIconCachedState`) and read from there on demand by the
    /// introspection extraction — nothing to record here. The redraw is for the
    /// overview's hover card, which paints whatever the surface currently names.
    impl XdgToplevelIconHandler for Dispatch {
        fn set_icon(&mut self, _toplevel: XdgToplevel, _wl_surface: WlSurface) {
            self.schedule_redraw();
        }
    }
    impl ShmHandler for Dispatch {
        fn shm_state(&self) -> &ShmState { &self.shm.state }
    }
    /// Decoration requests all arrive BEFORE the initial configure for the clients that
    /// matter here (Chromium sets its mode ~16ms before we configure). Sending a configure
    /// from these handlers is what produced the configure→reconfigure pair: the mode
    /// request emitted sequence #1, the initial configure emitted an identical sequence #2,
    /// and Chromium's xx-session-management code CHECK-crashes when a second
    /// `xdg_toplevel.configure` is dispatched before it has acked the first.
    ///
    /// So: record the mode only, and let the initial configure carry it. Once the client is
    /// configured (a mode toggle on a live window) the configure is sent as before.
    impl XdgDecorationHandler for Dispatch {
        fn new_decoration(&mut self, toplevel: ToplevelSurface) {
            toplevel.with_pending_state(|state| { state.decoration_mode = Some(Mode::ServerSide); });
            if toplevel.is_initial_configure_sent() { toplevel.send_pending_configure(); }
        }
        fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
            toplevel.with_pending_state(|state| { state.decoration_mode = Some(mode); });
            if toplevel.is_initial_configure_sent() { toplevel.send_pending_configure(); }
        }
        fn unset_mode(&mut self, toplevel: ToplevelSurface) {
            toplevel.with_pending_state(|state| { state.decoration_mode = Some(Mode::ServerSide); });
            if toplevel.is_initial_configure_sent() { toplevel.send_pending_configure(); }
        }
    }

    // Silence unused-import for SeatState (referenced via SeatHandler in state.rs).
    #[allow(dead_code)]
    fn _seatstate_marker(_: &SeatState<Dispatch>) {}
}
