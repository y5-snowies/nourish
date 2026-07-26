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
    // Deferred `set_data_device_focus` (needs DataDeviceHandler — downstream);
    // recorded by `focus_changed`, applied by the rim drain. The inner
    // `Option<Client>` is the focused client (None == clear focus).
    pub pending_data_focus: Option<Option<Client>>,
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
        self.needs_redraw = true;
    }

    /// Ungated re-arm for the lost-wakeup guard: a ping consumed the latch while
    /// a flip was in flight, so that pipe's pending vblank still needs to find
    /// `needs_redraw` set. Bounded by definition (it only fires with a flip
    /// actually in flight), so exclusive pacing leaves it alone.
    #[inline]
    pub fn rearm_redraw(&mut self) { self.needs_redraw = true; }
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
    /// frame — they are the cadence.
    #[inline]
    pub fn schedule_redraw_unchecked(&mut self) {
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
            // Activate-on-focus is DEFERRED to the drain. This callback can re-enter from
            // INSIDE `pointer.motion` (a popup-grab teardown restores keyboard focus while
            // motion holds the pointer's mutex); `is_pointer_over` → `pointer.current_focus()`
            // would re-lock that same non-reentrant mutex → deadlock. The drain runs with the
            // pointer unlocked, so the pointer-over check + constraint activate are safe there.
            self.pending_constraint_activation = focused.cloned();
            self.seat.previous_focus = focused.cloned();
        }

        // `set_data_device_focus` needs DataDeviceHandler (downstream) — defer it.
        self.pending_data_focus = Some(client);

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
    use smithay::backend::allocator::dmabuf::Dmabuf;
    use smithay::backend::renderer::utils::on_commit_buffer_handler;
    use smithay::desktop::{PopupKind, PopupManager, WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output};
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
                    // Each gate asks a different question of the surface, so all
                    // three properties are resolved here rather than assuming the
                    // tag: `Focused` admits the focused window whether or not it
                    // is tagged, and `Visible` admits anything the scene drew.
                    let focus = self
                        .seat
                        .seat
                        .get_keyboard()
                        .and_then(|kb| kb.current_focus());
                    let focused = focus.is_some_and(|f| &f == surface);
                    let frame = gate::frame();
                    let (tagged, visible) = compositor::with_states(surface, |states| {
                        (
                            states
                                .data_map
                                .get::<pacer::PacerSurface>()
                                .is_some_and(|t| t.get()),
                            states
                                .data_map
                                .get::<gate::VisibleSurface>()
                                .is_some_and(|v| v.fresh(frame)),
                        )
                    });
                    // `_unchecked` because `schedule_redraw` is itself gated while
                    // engaged — this IS the cadence, so it must bypass the gate.
                    if g.admits(tagged, focused, visible) {
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
