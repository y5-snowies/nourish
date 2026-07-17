//! The layer surface under test ("subject").
//!
//! Owns a single `zwlr_layer_surface_v1` (via sctk's `wlr_layer` helper, which auto-acks
//! configure) and reconfigures it on command: layer level, anchors, exclusive zone, margins,
//! keyboard interactivity, size, popups, input/opaque regions, output binding, buffer scale,
//! transparency and animation. It reads one [`Command`] per line on stdin (from the
//! controller) and renders a live state overlay + a pointer crosshair + a click marker (to see
//! how input regions route pointer events) + keyboard focus/last-key (to see keyboard
//! interactivity).
//!
//! It ALSO acts as a **foreign-toplevel client**, binding BOTH the wlr manager
//! (`zwlr_foreign_toplevel_manager_v1`, list + control) and the ext list
//! (`ext_foreign_toplevel_list_v1`, list-only). It shows what each advertises side by side and
//! can issue the wlr handle's control requests (activate/close/maximize/minimize/fullscreen)
//! on a selected toplevel — exercising the compositor's just-added foreign-protocol opt-in.

use std::io::BufRead;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_subcompositor,
    output::{OutputHandler, OutputState},
    reexports::calloop::{
        EventLoop,
        channel::{Channel, Event as ChanEvent, channel},
        timer::{TimeoutAction, Timer},
    },
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor as LayerAnchor, KeyboardInteractivity, Layer as WlrLayer, LayerShell,
            LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
    subcompositor::SubcompositorState,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child,
    globals::{GlobalList, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_keyboard::WlKeyboard,
        wl_output::{self, WlOutput},
        wl_pointer::WlPointer,
        wl_region::WlRegion,
        wl_seat::WlSeat,
        wl_shm::Format,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::{
    ext::foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    xdg::shell::client::{
        xdg_popup::{self, XdgPopup},
        xdg_positioner::{self, XdgPositioner},
        xdg_surface::{self, XdgSurface},
        xdg_wm_base::{self, XdgWmBase},
    },
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use layer_stress::canvas::{Canvas, color};
use layer_stress::diag;
use layer_stress::protocol::{Anchor, Command, ForeignCtl, Kbd, Layer, Region, ScaleReq};
use layer_stress::{font, info, warn};

// ----------------------------------------------------------------------------------------
// Dispatch userdata markers
// ----------------------------------------------------------------------------------------

/// Userdata for our popup `xdg_surface` (carries the popup id).
#[derive(Clone, Copy, Debug)]
struct XdgSurfData(u32);
/// Userdata for our popup `xdg_popup`.
#[derive(Clone, Copy, Debug)]
struct PopupTag(u32);

// ----------------------------------------------------------------------------------------
// Popup + foreign bookkeeping
// ----------------------------------------------------------------------------------------

struct PopupEntry {
    id: u32,
    surface: WlSurface,
    xdg_surface: XdgSurface,
    popup: XdgPopup,
    w: i32,
    h: i32,
    anchor: Anchor,
    gravity: Anchor,
    off: (i32, i32),
    configured: bool,
    serial: Option<u32>,
    color: u32,
}

/// One toplevel advertised by the wlr manager (list + control side).
struct WlrTop {
    handle: ZwlrForeignToplevelHandleV1,
    title: String,
    app_id: String,
    states: Vec<u32>,
}

/// One toplevel advertised by the ext list (read-only side).
struct ExtTop {
    handle: ExtForeignToplevelHandleV1,
    identifier: String,
    title: String,
    app_id: String,
}

/// wlr `state` enum values (protocol-fixed).
const ST_MAXIMIZED: u32 = 0;
const ST_MINIMIZED: u32 = 1;
const ST_ACTIVATED: u32 = 2;
const ST_FULLSCREEN: u32 = 3;

/// A transient "you clicked here" marker; confirms which surface the compositor routed a click
/// to (and thus whether the input region let it through).
struct ClickFx {
    surface: WlSurface,
    x: f64,
    y: f64,
    ttl: u32,
    button: u32,
}
const CLICK_TTL: u32 = 18;

// ----------------------------------------------------------------------------------------
// Subject state
// ----------------------------------------------------------------------------------------

struct Subject {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    #[allow(dead_code)]
    subcompositor: SubcompositorState,
    layer_shell: LayerShell,
    qh: QueueHandle<Subject>,

    wm_base: XdgWmBase,
    viewporter: Option<WpViewporter>,
    frac_mgr: Option<WpFractionalScaleManagerV1>,

    // The layer surface + its per-surface objects (rebuilt on recreate/output change).
    layer: Option<LayerSurface>,
    viewport: Option<WpViewport>,
    _frac: Option<WpFractionalScaleV1>,

    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    seat: Option<WlSeat>,

    // layer params
    p_layer: Layer,
    anchor_bits: u32, // TOP=1 BOTTOM=2 LEFT=4 RIGHT=8 (wlr order)
    excl: i32,
    margins: (i32, i32, i32, i32), // t, r, b, l
    kbd: Kbd,
    req_size: (u32, u32),
    output_sel: usize,

    // render / scale
    configured: bool,
    cfg_size: (i32, i32),
    scale: ScaleReq,
    output_scale: i32,
    preferred_scale: u32, // x120, 0 = unknown
    input_region: Region,
    opaque: bool,
    transparent: bool,
    animate: bool,
    anim_phase: i32,

    // keyboard visualisation
    kbd_focus: bool,
    last_key: String,

    // popups
    popups: Vec<PopupEntry>,
    next_popup_id: u32,
    pop_anchor: Anchor,
    pop_gravity: Anchor,
    pop_off: (i32, i32),
    pop_size: (i32, i32),

    // foreign toplevel
    wlr_mgr: Option<ZwlrForeignToplevelManagerV1>,
    ext_list: Option<ExtForeignToplevelListV1>,
    wlr_tops: Vec<WlrTop>,
    ext_tops: Vec<ExtTop>,
    foreign_sel: usize,

    ptr: Option<(WlSurface, f64, f64)>,
    click: Option<ClickFx>,
    exit: bool,
}

const DEFAULT_W: i32 = 480;
const DEFAULT_H: i32 = 320;
const POPUP_COLORS: [u32; 4] = [color::BLUE, color::RED, color::GREEN, color::MAGENTA];

fn main() {
    diag::set_role("subject");

    let args: Vec<String> = std::env::args().collect();
    let use_vp = !args.iter().any(|a| a == "--no-viewporter");
    let use_fs = !args.iter().any(|a| a == "--no-fractional-scale");
    let use_fwlr = !args.iter().any(|a| a == "--no-foreign-wlr");
    let use_fext = !args.iter().any(|a| a == "--no-foreign-ext");

    let conn = Connection::connect_to_env().expect("connect to wayland");
    let (globals, event_queue) = registry_queue_init::<Subject>(&conn).expect("registry init");
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<Subject> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .expect("insert wayland source");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let subcompositor =
        SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &qh)
            .expect("wl_subcompositor");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let pool = SlotPool::new((DEFAULT_W * DEFAULT_H * 4) as usize, &shm).expect("slot pool");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("zwlr_layer_shell_v1");

    let wm_base: XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
    let viewporter: Option<WpViewporter> =
        if use_vp { bind_opt(&globals, &qh, "wp_viewporter", 1..=1) } else { None };
    let frac_mgr: Option<WpFractionalScaleManagerV1> =
        if use_fs { bind_opt(&globals, &qh, "wp_fractional_scale_manager_v1", 1..=1) } else { None };

    // Foreign toplevel: bind BOTH managers (advertisement check). Absence => compositor did
    // not advertise the global at all; empty list while bound => advertised but muted (the
    // `protocol_foreign` preference gate is off).
    let wlr_mgr: Option<ZwlrForeignToplevelManagerV1> =
        if use_fwlr { bind_opt(&globals, &qh, "zwlr_foreign_toplevel_manager_v1", 1..=3) } else { None };
    let ext_list: Option<ExtForeignToplevelListV1> =
        if use_fext { bind_opt(&globals, &qh, "ext_foreign_toplevel_list_v1", 1..=1) } else { None };

    info!(
        "globals: layer_shell + viewporter={} fractional={} foreign_wlr={} foreign_ext={}",
        viewporter.is_some(),
        frac_mgr.is_some(),
        wlr_mgr.is_some(),
        ext_list.is_some()
    );

    let mut subject = Subject {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        subcompositor,
        layer_shell,
        qh: qh.clone(),
        wm_base,
        viewporter,
        frac_mgr,
        layer: None,
        viewport: None,
        _frac: None,
        pointer: None,
        keyboard: None,
        seat: None,
        p_layer: Layer::Top,
        anchor_bits: LayerAnchor::TOP.bits() | LayerAnchor::LEFT.bits(),
        excl: 0,
        margins: (0, 0, 0, 0),
        kbd: Kbd::None,
        req_size: (DEFAULT_W as u32, DEFAULT_H as u32),
        output_sel: 0,
        configured: false,
        cfg_size: (DEFAULT_W, DEFAULT_H),
        scale: ScaleReq::Normal,
        output_scale: 1,
        preferred_scale: 0,
        input_region: Region::Full,
        opaque: false,
        transparent: false,
        animate: false,
        anim_phase: 0,
        kbd_focus: false,
        last_key: "-".into(),
        popups: Vec::new(),
        next_popup_id: 1,
        pop_anchor: Anchor::BottomLeft,
        pop_gravity: Anchor::BottomRight,
        pop_off: (0, 0),
        pop_size: (160, 120),
        wlr_mgr,
        ext_list,
        wlr_tops: Vec::new(),
        ext_tops: Vec::new(),
        foreign_sel: 0,
        ptr: None,
        click: None,
        exit: false,
    };

    // Build the initial layer surface.
    subject.create_layer();

    // stdin command channel: a blocking reader thread forwards lines into the loop.
    let (tx, rx): (_, Channel<String>) = channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(l) = line else { break };
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    event_loop
        .handle()
        .insert_source(rx, |event, _, state| match event {
            ChanEvent::Msg(line) => state.on_line(&line),
            ChanEvent::Closed => {
                info!("stdin closed; exiting");
                state.exit = true;
            }
        })
        .expect("insert stdin channel");

    // ~60Hz timer drives animation + the click marker.
    event_loop
        .handle()
        .insert_source(
            Timer::from_duration(std::time::Duration::from_millis(16)),
            |_, _, state| {
                state.tick();
                TimeoutAction::ToDuration(std::time::Duration::from_millis(16))
            },
        )
        .expect("insert timer");

    info!("subject ready; waiting for configure");
    loop {
        event_loop.dispatch(std::time::Duration::from_millis(50), &mut subject).unwrap();
        if subject.exit {
            info!("subject exiting");
            break;
        }
    }
}

/// Bind an optional global by name over a version range, logging absence.
fn bind_opt<I>(
    globals: &GlobalList,
    qh: &QueueHandle<Subject>,
    name: &str,
    range: std::ops::RangeInclusive<u32>,
) -> Option<I>
where
    I: Proxy + 'static,
    Subject: Dispatch<I, ()>,
{
    match globals.bind::<I, Subject, ()>(qh, range, ()) {
        Ok(p) => {
            info!("bound global {name} v{}", p.version());
            Some(p)
        }
        Err(e) => {
            warn!("global {name} unavailable: {e}");
            None
        }
    }
}

impl Subject {
    // ---- layer surface lifecycle ----------------------------------------------------

    /// Build a fresh layer surface (+ viewport/fractional-scale objects) on the selected
    /// output and push every current parameter to it.
    fn create_layer(&mut self) {
        let qh = self.qh.clone();
        let surface = self.compositor.create_surface(&qh);

        let outputs: Vec<WlOutput> = self.output_state.outputs().collect();
        let output = outputs.get(self.output_sel).cloned();

        let layer = self.layer_shell.create_layer_surface(
            &qh,
            surface,
            to_wlr_layer(self.p_layer),
            Some("y5.layer.stress"),
            output.as_ref(),
        );

        self.viewport = self.viewporter.as_ref().map(|v| v.get_viewport(layer.wl_surface(), &qh, ()));
        self._frac = self.frac_mgr.as_ref().map(|m| m.get_fractional_scale(layer.wl_surface(), &qh, ()));

        self.configured = false;
        self.layer = Some(layer);
        self.apply_layer_params();
        if let Some(l) = &self.layer {
            l.commit();
        }
        info!(
            "created layer surface: layer={:?} output={} anchor={:#06b}",
            self.p_layer, self.output_sel, self.anchor_bits
        );
    }

    fn destroy_layer(&mut self) {
        for p in self.popups.drain(..) {
            p.popup.destroy();
            p.xdg_surface.destroy();
            p.surface.destroy();
        }
        self.viewport.take();
        self._frac.take();
        self.layer.take(); // dropping the LayerSurface destroys the wlr object + surface
    }

    fn recreate(&mut self) {
        self.destroy_layer();
        self.create_layer();
    }

    /// Push anchor / exclusive zone / margins / keyboard / size to the current layer surface.
    fn apply_layer_params(&mut self) {
        let Some(l) = &self.layer else { return };
        l.set_anchor(LayerAnchor::from_bits_truncate(self.anchor_bits));
        l.set_exclusive_zone(self.excl);
        let (t, r, b, ln) = self.margins;
        l.set_margin(t, r, b, ln);
        l.set_keyboard_interactivity(to_kbd(self.kbd));
        l.set_size(self.req_size.0, self.req_size.1);
    }

    // ---- command handling -----------------------------------------------------------

    fn on_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        match Command::parse(line) {
            Some(cmd) => {
                info!("cmd: {}", cmd.encode());
                self.apply(cmd);
            }
            None => warn!("unparsed command: {line:?}"),
        }
    }

    fn apply(&mut self, cmd: Command) {
        let mut reconfigure = true;
        match cmd {
            Command::SetLayer(l) => {
                self.p_layer = l;
                // set_layer is live (wlr v2+); fall back to recreate is unnecessary on smithay.
                if let Some(s) = &self.layer {
                    s.set_layer(to_wlr_layer(l));
                }
            }
            Command::AnchorTop => self.anchor_bits ^= LayerAnchor::TOP.bits(),
            Command::AnchorBottom => self.anchor_bits ^= LayerAnchor::BOTTOM.bits(),
            Command::AnchorLeft => self.anchor_bits ^= LayerAnchor::LEFT.bits(),
            Command::AnchorRight => self.anchor_bits ^= LayerAnchor::RIGHT.bits(),
            Command::AnchorAll => {
                self.anchor_bits =
                    LayerAnchor::TOP.bits() | LayerAnchor::BOTTOM.bits() | LayerAnchor::LEFT.bits() | LayerAnchor::RIGHT.bits();
            }
            Command::AnchorNone => self.anchor_bits = 0,

            Command::Excl(n) => self.excl = n,
            Command::Margin(t, r, b, l) => self.margins = (t, r, b, l),
            Command::Keyboard(k) => self.kbd = k,
            Command::Size(w, h) => self.req_size = (w, h),

            Command::PopupAdd => {
                self.add_popup();
                reconfigure = false;
            }
            Command::PopupClose => {
                if let Some(p) = self.popups.pop() {
                    p.popup.destroy();
                    p.xdg_surface.destroy();
                    p.surface.destroy();
                }
                reconfigure = false;
            }
            Command::PopupAnchor(a) => self.pop_anchor = a,
            Command::PopupGravity(g) => self.pop_gravity = g,
            Command::PopupOff(x, y) => self.pop_off = (x, y),
            Command::PopupSize(w, h) => self.pop_size = (w as i32, h as i32),
            Command::PopupMove(dx, dy) => {
                self.reposition_last_popup(dx, dy);
                reconfigure = false;
            }

            Command::Input(r) => {
                self.input_region = r;
                self.apply_input_region();
                reconfigure = false;
            }
            Command::Opaque(on) => {
                self.opaque = on;
                self.apply_opaque_region();
                reconfigure = false;
            }

            Command::OutputCycle => {
                let n = self.output_state.outputs().count().max(1);
                self.output_sel = (self.output_sel + 1) % n;
                self.recreate();
                reconfigure = false;
            }

            Command::Scale(s) => self.scale = s,
            Command::Transparent(on) => self.transparent = on,
            Command::Animate(on) => self.animate = on,

            Command::ForeignSel(d) => {
                let n = self.wlr_tops.len();
                if n > 0 {
                    let i = self.foreign_sel as i64 + d as i64;
                    self.foreign_sel = i.rem_euclid(n as i64) as usize;
                }
                reconfigure = false;
            }
            Command::Foreign(c) => {
                self.foreign_control(c);
                reconfigure = false;
            }

            Command::Recreate => {
                self.recreate();
                reconfigure = false;
            }
            Command::Quit => {
                self.exit = true;
                reconfigure = false;
            }
        }
        if reconfigure {
            self.apply_layer_params();
        }
        if self.configured {
            self.draw();
        }
    }

    // ---- popups ---------------------------------------------------------------------

    fn add_popup(&mut self) {
        let Some(layer) = &self.layer else { return };
        let qh = self.qh.clone();
        let id = self.next_popup_id;
        self.next_popup_id += 1;

        let (pw, ph) = self.pop_size;
        let (lw, lh) = self.cfg_size;

        let positioner = self.wm_base.create_positioner(&qh, ());
        positioner.set_size(pw.max(1), ph.max(1));
        positioner.set_anchor_rect(0, 0, lw.max(1), lh.max(1));
        positioner.set_anchor(to_anchor(self.pop_anchor));
        positioner.set_gravity(to_gravity(self.pop_gravity));
        positioner.set_offset(self.pop_off.0, self.pop_off.1);
        positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::all());

        let surface = self.compositor.create_surface(&qh);
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &qh, XdgSurfData(id));
        // Layer-shell popup: no xdg parent; the layer surface becomes the parent via get_popup.
        let popup = xdg_surface.get_popup(None, &positioner, &qh, PopupTag(id));
        layer.get_popup(&popup);
        positioner.destroy();
        surface.commit();

        info!("popup {id} anchor={:?} gravity={:?} off={:?} size={pw}x{ph}", self.pop_anchor, self.pop_gravity, self.pop_off);

        self.popups.push(PopupEntry {
            id,
            surface,
            xdg_surface,
            popup,
            w: pw.max(1),
            h: ph.max(1),
            anchor: self.pop_anchor,
            gravity: self.pop_gravity,
            off: self.pop_off,
            configured: false,
            serial: None,
            color: POPUP_COLORS[(id as usize) % POPUP_COLORS.len()],
        });
    }

    fn reposition_last_popup(&mut self, dx: i32, dy: i32) {
        let qh = self.qh.clone();
        let (lw, lh) = self.cfg_size;
        if let Some(p) = self.popups.last_mut() {
            p.off = (p.off.0 + dx, p.off.1 + dy);
            if p.popup.version() >= 3 {
                let positioner = self.wm_base.create_positioner(&qh, ());
                positioner.set_size(p.w.max(1), p.h.max(1));
                positioner.set_anchor_rect(0, 0, lw.max(1), lh.max(1));
                positioner.set_anchor(to_anchor(p.anchor));
                positioner.set_gravity(to_gravity(p.gravity));
                positioner.set_offset(p.off.0, p.off.1);
                positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::all());
                p.popup.reposition(&positioner, p.id);
                positioner.destroy();
                info!("popup {} reposition off={:?}", p.id, p.off);
            } else {
                warn!("popup reposition needs xdg v3; have v{}", p.popup.version());
            }
        }
    }

    // ---- regions --------------------------------------------------------------------

    fn apply_input_region(&mut self) {
        let qh = self.qh.clone();
        let Some(layer) = &self.layer else { return };
        let (w, h) = (self.cfg_size.0.max(1), self.cfg_size.1.max(1));
        let wl = layer.wl_surface().clone();
        match self.input_region {
            Region::Full => wl.set_input_region(None), // null == whole surface
            Region::None => {
                let region = self.compositor.wl_compositor().create_region(&qh, ());
                wl.set_input_region(Some(&region)); // empty region == click-through
                region.destroy();
            }
            Region::Circle => {
                let region = self.compositor.wl_compositor().create_region(&qh, ());
                // Rectangle-approximated disc centred in the surface.
                let (cx, cy, r) = (w / 2, h / 2, w.min(h) / 2);
                let steps = 16;
                for i in 0..steps {
                    let yy = -r + (2 * r) * i / steps;
                    let yy2 = -r + (2 * r) * (i + 1) / steps;
                    let ymid = (yy + yy2) / 2;
                    let half = (((r * r - ymid * ymid).max(0)) as f64).sqrt() as i32;
                    region.add(cx - half, cy + yy, half * 2, (yy2 - yy).max(1));
                }
                wl.set_input_region(Some(&region));
                region.destroy();
            }
            Region::Holes => {
                let region = self.compositor.wl_compositor().create_region(&qh, ());
                region.add(0, 0, w, h);
                // Subtract a centred hole -> clicks in the middle pass through.
                region.subtract(w / 4, h / 4, w / 2, h / 2);
                wl.set_input_region(Some(&region));
                region.destroy();
            }
        }
        wl.commit();
        info!("input region -> {:?}", self.input_region);
    }

    fn apply_opaque_region(&mut self) {
        let qh = self.qh.clone();
        let Some(layer) = &self.layer else { return };
        let (w, h) = (self.cfg_size.0.max(1), self.cfg_size.1.max(1));
        let wl = layer.wl_surface().clone();
        if self.opaque {
            let region = self.compositor.wl_compositor().create_region(&qh, ());
            region.add(0, 0, w, h);
            wl.set_opaque_region(Some(&region));
            region.destroy();
        } else {
            wl.set_opaque_region(None);
        }
        wl.commit();
        info!("opaque region -> {}", if self.opaque { "full" } else { "none" });
    }

    // ---- foreign toplevel control ---------------------------------------------------

    fn foreign_control(&mut self, c: ForeignCtl) {
        let Some(top) = self.wlr_tops.get(self.foreign_sel) else {
            warn!("foreign control: no toplevel selected");
            return;
        };
        let h = &top.handle;
        match c {
            ForeignCtl::Activate => match &self.seat {
                Some(seat) => h.activate(seat),
                None => warn!("foreign activate: no seat"),
            },
            ForeignCtl::Close => h.close(),
            ForeignCtl::Maximized(true) => h.set_maximized(),
            ForeignCtl::Maximized(false) => h.unset_maximized(),
            ForeignCtl::Minimized(true) => h.set_minimized(),
            ForeignCtl::Minimized(false) => h.unset_minimized(),
            ForeignCtl::Fullscreen(on) => {
                if h.version() >= 2 {
                    if on {
                        h.set_fullscreen(None);
                    } else {
                        h.unset_fullscreen();
                    }
                } else {
                    warn!("foreign fullscreen needs wlr handle v2; have v{}", h.version());
                }
            }
        }
        info!("foreign {:?} -> {} ({})", c, top.title, top.app_id);
    }

    // ---- tick + redraw --------------------------------------------------------------

    fn tick(&mut self) {
        if self.animate && self.configured {
            self.anim_phase = (self.anim_phase + 3) % 360;
            self.draw();
        }
        let expiring = match &mut self.click {
            Some(fx) if fx.ttl > 0 => {
                fx.ttl -= 1;
                Some(false)
            }
            Some(_) => Some(true),
            None => None,
        };
        if let Some(done) = expiring {
            if done {
                self.click = None;
            }
            if self.configured {
                self.draw();
            }
        }
    }

    // ---- drawing --------------------------------------------------------------------

    fn draw(&mut self) {
        if !self.configured || self.layer.is_none() {
            return;
        }
        let (lw, lh) = (self.cfg_size.0.max(1), self.cfg_size.1.max(1));
        let (bw, bh, bscale, dest) = self.resolve_scale(lw, lh);

        let overlay = self.overlay_lines(lw, lh, bw, bh, bscale);
        let bg = if self.transparent { 0x80101418 } else { 0xFF101418 };
        let anim_x = if self.animate { (self.anim_phase * bw / 360).clamp(0, bw - 3) } else { -10 };
        let ptr_local = self
            .ptr
            .as_ref()
            .filter(|(s, _, _)| Some(s) == self.layer.as_ref().map(|l| l.wl_surface()))
            .map(|(_, x, y)| (*x, *y));
        let click_local = self
            .click
            .as_ref()
            .filter(|fx| Some(&fx.surface) == self.layer.as_ref().map(|l| l.wl_surface()))
            .map(|fx| (fx.x, fx.y, fx.ttl, fx.button));
        let sx = bw as f32 / lw as f32;
        let sy = bh as f32 / lh as f32;

        let stride = bw * 4;
        let (buffer, slice) =
            self.pool.create_buffer(bw, bh, stride, Format::Argb8888).expect("create buffer");
        {
            let mut cv = Canvas::new(slice, bw, bh);
            cv.clear(bg);
            cv.frame(0, 0, bw, bh, 2, color::CYAN);
            if anim_x >= 0 {
                cv.rect(anim_x, 0, 3, bh, color::YELLOW);
            }
            let mut yy = 8;
            for s in &overlay {
                font::text(&mut cv, 8, yy, 1, color::LTGREY, s);
                yy += 11;
            }
            // Logical -> buffer-pixel mapping for markers (viewport dest or buffer_scale).
            let to_buf = |px: f64, py: f64| {
                let (vw, vh) = match dest {
                    Some((dw, dh)) => (dw.max(1) as f32, dh.max(1) as f32),
                    None => {
                        let bs = bscale.max(1) as f32;
                        (bw as f32 / bs, bh as f32 / bs)
                    }
                };
                ((px as f32 * bw as f32 / vw) as i32, (py as f32 * bh as f32 / vh) as i32)
            };
            if let Some((px, py)) = ptr_local {
                let (x, y) = to_buf(px, py);
                cv.crosshair(x, y, 12, color::YELLOW);
                font::text(&mut cv, x + 8, y + 8, 1, color::YELLOW, &format!("{px:.0},{py:.0}"));
            }
            if let Some((px, py, ttl, button)) = click_local {
                let (x, y) = to_buf(px, py);
                click_marker(&mut cv, x, y, ttl, button);
            }
            let _ = (sx, sy);
        }

        let wl = self.layer.as_ref().unwrap().wl_surface().clone();
        wl.set_buffer_scale(bscale);
        if let Some(v) = &self.viewport {
            match dest {
                Some((dw, dh)) => v.set_destination(dw.max(1), dh.max(1)),
                None => v.set_destination(-1, -1),
            }
        }
        wl.damage_buffer(0, 0, bw, bh);
        buffer.attach_to(&wl).expect("attach");
        wl.commit();
    }

    fn overlay_lines(&self, lw: i32, lh: i32, bw: i32, bh: i32, bscale: i32) -> Vec<String> {
        let (t, r, b, l) = self.margins;
        let mut v = vec![
            format!(
                "LAYER {:?}  ANCHOR {}",
                self.p_layer,
                anchor_str(self.anchor_bits)
            ),
            format!("EXCL {}  MARGIN {t} {r} {b} {l}", self.excl),
            format!(
                "SIZE req {}x{} -> cfg {lw}x{lh}  KBD {:?} FOCUS {}",
                self.req_size.0,
                self.req_size.1,
                self.kbd,
                if self.kbd_focus { "YES" } else { "no" }
            ),
            format!("KEY {}", self.last_key),
            format!(
                "BUF {bw}x{bh} scale {bscale}  OUTSCALE {} FRAC {}  SCALE {:?}",
                self.output_scale,
                frac_str(self.preferred_scale),
                self.scale
            ),
            format!(
                "INPUT {:?}  OPAQUE {}  OUTPUT {}/{}  POPUPS {}  TRANSP {} ANIM {}",
                self.input_region,
                if self.opaque { "full" } else { "none" },
                self.output_sel,
                self.output_state.outputs().count(),
                self.popups.len(),
                on(self.transparent),
                on(self.animate),
            ),
            format!(
                "FOREIGN wlr={} ext={}  SEL {}",
                present(self.wlr_mgr.as_ref().map(|m| m.version())),
                present(self.ext_list.as_ref().map(|m| m.version())),
                self.foreign_sel,
            ),
        ];
        for (i, t) in self.wlr_tops.iter().enumerate().take(6) {
            let sel = if i == self.foreign_sel { ">" } else { " " };
            v.push(format!(
                "{sel}WLR[{i}] {} ({}) [{}]",
                trunc(&t.title, 18),
                trunc(&t.app_id, 14),
                states_str(&t.states)
            ));
        }
        for (i, t) in self.ext_tops.iter().enumerate().take(4) {
            v.push(format!(
                " EXT[{i}] {} ({}) id={}",
                trunc(&t.title, 16),
                trunc(&t.app_id, 12),
                trunc(&t.identifier, 10)
            ));
        }
        v
    }

    /// Buffer pixel size + buffer scale + optional viewport destination for the scale mode.
    fn resolve_scale(&self, lw: i32, lh: i32) -> (i32, i32, i32, Option<(i32, i32)>) {
        let os = self.output_scale.max(1);
        let frac = if self.preferred_scale > 0 { self.preferred_scale as f32 / 120.0 } else { 1.0 };
        let r = |v: f32| v.round().max(1.0) as i32;
        match self.scale {
            ScaleReq::Normal => (lw, lh, 1, None),
            ScaleReq::FsHonor => (r(lw as f32 * frac), r(lh as f32 * frac), 1, Some((lw, lh))),
            ScaleReq::FsIgnore => (lw, lh, 1, Some((lw, lh))),
            ScaleReq::DpiHonor => (lw * os, lh * os, os, None),
            ScaleReq::DpiIgnore => (lw, lh, 1, None),
            ScaleReq::DpiScale(n) => {
                let s = n.max(1);
                (lw * s, lh * s, s, None)
            }
        }
    }

    // ---- foreign-toplevel bookkeeping helpers ---------------------------------------

    fn wlr_top_mut(&mut self, h: &ZwlrForeignToplevelHandleV1) -> Option<&mut WlrTop> {
        self.wlr_tops.iter_mut().find(|t| &t.handle == h)
    }
    fn ext_top_mut(&mut self, h: &ExtForeignToplevelHandleV1) -> Option<&mut ExtTop> {
        self.ext_tops.iter_mut().find(|t| &t.handle == h)
    }
}

// ---- small helpers -----------------------------------------------------------------------

fn button_color(button: u32) -> u32 {
    match button {
        0x110 => color::RED,
        0x111 => color::GREEN,
        0x112 => color::CYAN,
        _ => color::MAGENTA,
    }
}

fn click_marker(cv: &mut Canvas, cx: i32, cy: i32, ttl: u32, button: u32) {
    let rad = 3 + (CLICK_TTL - ttl.min(CLICK_TTL)) as i32;
    cv.ring(cx, cy, rad + 1, color::BLACK);
    cv.ring(cx, cy, rad, color::WHITE);
    cv.rect(cx - 2, cy - 2, 5, 5, button_color(button));
    cv.frame(cx - 2, cy - 2, 5, 5, 1, color::BLACK);
}

fn anchor_str(bits: u32) -> String {
    let mut s = String::new();
    s.push(if bits & LayerAnchor::TOP.bits() != 0 { 'T' } else { '-' });
    s.push(if bits & LayerAnchor::BOTTOM.bits() != 0 { 'B' } else { '-' });
    s.push(if bits & LayerAnchor::LEFT.bits() != 0 { 'L' } else { '-' });
    s.push(if bits & LayerAnchor::RIGHT.bits() != 0 { 'R' } else { '-' });
    s
}

fn states_str(states: &[u32]) -> String {
    let mut s = String::new();
    for st in states {
        s.push(match *st {
            ST_MAXIMIZED => 'M',
            ST_MINIMIZED => 'm',
            ST_ACTIVATED => 'A',
            ST_FULLSCREEN => 'F',
            _ => '?',
        });
    }
    if s.is_empty() {
        s.push('-');
    }
    s
}

fn decode_states(bytes: &[u8]) -> Vec<u32> {
    bytes.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn present(v: Option<u32>) -> String {
    match v {
        Some(ver) => format!("v{ver}"),
        None => "absent".into(),
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

fn on(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn frac_str(x120: u32) -> String {
    if x120 == 0 { "?".into() } else { format!("{:.2}", x120 as f32 / 120.0) }
}

fn to_wlr_layer(l: Layer) -> WlrLayer {
    match l {
        Layer::Background => WlrLayer::Background,
        Layer::Bottom => WlrLayer::Bottom,
        Layer::Top => WlrLayer::Top,
        Layer::Overlay => WlrLayer::Overlay,
    }
}

fn to_kbd(k: Kbd) -> KeyboardInteractivity {
    match k {
        Kbd::None => KeyboardInteractivity::None,
        Kbd::Exclusive => KeyboardInteractivity::Exclusive,
        Kbd::OnDemand => KeyboardInteractivity::OnDemand,
    }
}

fn to_anchor(a: Anchor) -> xdg_positioner::Anchor {
    use xdg_positioner::Anchor as A;
    match a {
        Anchor::Center => A::None,
        Anchor::Top => A::Top,
        Anchor::Bottom => A::Bottom,
        Anchor::Left => A::Left,
        Anchor::Right => A::Right,
        Anchor::TopLeft => A::TopLeft,
        Anchor::TopRight => A::TopRight,
        Anchor::BottomLeft => A::BottomLeft,
        Anchor::BottomRight => A::BottomRight,
    }
}
fn to_gravity(a: Anchor) -> xdg_positioner::Gravity {
    use xdg_positioner::Gravity as G;
    match a {
        Anchor::Center => G::None,
        Anchor::Top => G::Top,
        Anchor::Bottom => G::Bottom,
        Anchor::Left => G::Left,
        Anchor::Right => G::Right,
        Anchor::TopLeft => G::TopLeft,
        Anchor::TopRight => G::TopRight,
        Anchor::BottomLeft => G::BottomLeft,
        Anchor::BottomRight => G::BottomRight,
    }
}

// ==========================================================================================
// sctk handler impls
// ==========================================================================================

impl LayerShellHandler for Subject {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        info!("layer surface closed by compositor");
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _: u32,
    ) {
        if Some(layer.wl_surface()) != self.layer.as_ref().map(|l| l.wl_surface()) {
            return;
        }
        let (mut w, mut h) = (cfg.new_size.0 as i32, cfg.new_size.1 as i32);
        // A zero dimension means "pick your own": fall back to the requested size, then default.
        if w == 0 {
            w = if self.req_size.0 > 0 { self.req_size.0 as i32 } else { DEFAULT_W };
        }
        if h == 0 {
            h = if self.req_size.1 > 0 { self.req_size.1 as i32 } else { DEFAULT_H };
        }
        self.cfg_size = (w.max(1), h.max(1));
        self.configured = true;
        // Regions depend on the configured size — refresh them.
        self.apply_input_region();
        self.apply_opaque_region();
        self.draw();
    }
}

impl CompositorHandler for Subject {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, new: i32) {
        if Some(surface) == self.layer.as_ref().map(|l| l.wl_surface()) {
            self.output_scale = new.max(1);
            self.draw();
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        if Some(surface) == self.layer.as_ref().map(|l| l.wl_surface()) {
            self.draw();
        } else if let Some(i) = self.popups.iter().position(|p| &p.surface == surface) {
            self.draw_popup(i);
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
}

impl Subject {
    fn draw_popup(&mut self, idx: usize) {
        let (w, h) = (self.popups[idx].w.max(1), self.popups[idx].h.max(1));
        let color = self.popups[idx].color;
        let surface = self.popups[idx].surface.clone();
        let serial = self.popups[idx].serial.take();
        let id = self.popups[idx].id;
        let ptr_local = self
            .ptr
            .as_ref()
            .filter(|(s, _, _)| s == &surface)
            .map(|(_, x, y)| (*x, *y));
        let stride = w * 4;
        let (buffer, slice) =
            self.pool.create_buffer(w, h, stride, Format::Argb8888).expect("popup buffer");
        {
            let mut cv = Canvas::new(slice, w, h);
            cv.clear(color);
            cv.frame(0, 0, w, h, 2, color::WHITE);
            font::text(&mut cv, 6, 6, 2, color::BLACK, &format!("POPUP {id}"));
            if let Some((px, py)) = ptr_local {
                cv.crosshair(px as i32, py as i32, 10, color::BLACK);
            }
        }
        if let Some(s) = serial {
            self.popups[idx].xdg_surface.ack_configure(s);
        }
        surface.damage_buffer(0, 0, w, h);
        buffer.attach_to(&surface).expect("attach popup");
        surface.commit();
    }
}

impl OutputHandler for Subject {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        if let Some(info) = self.output_state.info(&output) {
            self.output_scale = info.scale_factor.max(1);
        }
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl ShmHandler for Subject {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl SeatHandler for Subject {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.seat = Some(seat);
    }
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
        self.seat = Some(seat.clone());
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard::<Self, Self>(qh, &seat, None).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, cap: Capability) {
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if cap == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for Subject {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, surface: &WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        if Some(surface) == self.layer.as_ref().map(|l| l.wl_surface()) {
            self.kbd_focus = true;
            if self.configured {
                self.draw();
            }
        }
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, surface: &WlSurface, _: u32) {
        if Some(surface) == self.layer.as_ref().map(|l| l.wl_surface()) {
            self.kbd_focus = false;
            if self.configured {
                self.draw();
            }
        }
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, event: KeyEvent) {
        let utf8 = event.utf8.clone().unwrap_or_default();
        self.last_key = format!("code={} sym={:#x} {utf8}", event.raw_code, event.keysym.raw());
        info!("key press {}", self.last_key);
        if self.configured {
            self.draw();
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: Modifiers, _: u32) {}
}

impl PointerHandler for Subject {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlPointer, events: &[PointerEvent]) {
        for e in events {
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.ptr = Some((e.surface.clone(), e.position.0, e.position.1));
                }
                PointerEventKind::Leave { .. } => {
                    if self.ptr.as_ref().map(|(s, _, _)| s == &e.surface).unwrap_or(false) {
                        self.ptr = None;
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    self.click = Some(ClickFx {
                        surface: e.surface.clone(),
                        x: e.position.0,
                        y: e.position.1,
                        ttl: CLICK_TTL,
                        button,
                    });
                }
                _ => {}
            }
        }
        if self.configured {
            self.draw();
            for i in 0..self.popups.len() {
                if self.popups[i].configured {
                    self.draw_popup(i);
                }
            }
        }
    }
}

impl ProvidesRegistryState for Subject {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Subject);
delegate_subcompositor!(Subject);
delegate_output!(Subject);
delegate_shm!(Subject);
delegate_seat!(Subject);
delegate_pointer!(Subject);
delegate_keyboard!(Subject);
delegate_layer!(Subject);
delegate_registry!(Subject);

// ==========================================================================================
// Manual Dispatch impls for the raw objects
// ==========================================================================================

impl Dispatch<XdgWmBase, ()> for Subject {
    fn event(_: &mut Self, wm: &XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, XdgSurfData> for Subject {
    fn event(state: &mut Self, _: &XdgSurface, event: xdg_surface::Event, data: &XdgSurfData, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            if let Some(idx) = state.popups.iter().position(|p| p.id == data.0) {
                state.popups[idx].serial = Some(serial);
                state.popups[idx].configured = true;
                state.draw_popup(idx);
            }
        }
    }
}

impl Dispatch<XdgPopup, PopupTag> for Subject {
    fn event(state: &mut Self, _: &XdgPopup, event: xdg_popup::Event, tag: &PopupTag, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                if let Some(p) = state.popups.iter_mut().find(|p| p.id == tag.0) {
                    if width > 0 && height > 0 {
                        p.w = width;
                        p.h = height;
                    }
                }
            }
            xdg_popup::Event::PopupDone => {
                if let Some(i) = state.popups.iter().position(|p| p.id == tag.0) {
                    let p = state.popups.remove(i);
                    p.popup.destroy();
                    p.xdg_surface.destroy();
                    p.surface.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgPositioner, ()> for Subject {
    fn event(_: &mut Self, _: &XdgPositioner, _: xdg_positioner::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlRegion, ()> for Subject {
    fn event(_: &mut Self, _: &WlRegion, _: <WlRegion as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewporter, ()> for Subject {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WpViewport, ()> for Subject {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for Subject {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WpFractionalScaleV1, ()> for Subject {
    fn event(state: &mut Self, _: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.preferred_scale = scale;
            if state.configured {
                state.draw();
            }
        }
    }
}

// ---- foreign toplevel: wlr manager + handle ----------------------------------------------

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Subject {
    fn event(state: &mut Self, _: &ZwlrForeignToplevelManagerV1, event: zwlr_foreign_toplevel_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                info!("wlr foreign: new toplevel handle");
                state.wlr_tops.push(WlrTop {
                    handle: toplevel,
                    title: String::new(),
                    app_id: String::new(),
                    states: Vec::new(),
                });
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                info!("wlr foreign: manager finished");
                state.wlr_tops.clear();
                state.wlr_mgr = None;
            }
            _ => {}
        }
        if state.configured {
            state.draw();
        }
    }
    event_created_child!(Subject, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Subject {
    fn event(state: &mut Self, handle: &ZwlrForeignToplevelHandleV1, event: zwlr_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(t) = state.wlr_top_mut(handle) {
                    t.title = title;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(t) = state.wlr_top_mut(handle) {
                    t.app_id = app_id;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                let decoded = decode_states(&bytes);
                if let Some(t) = state.wlr_top_mut(handle) {
                    t.states = decoded;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.wlr_tops.retain(|t| &t.handle != handle);
                handle.destroy();
                if state.foreign_sel >= state.wlr_tops.len() {
                    state.foreign_sel = state.wlr_tops.len().saturating_sub(1);
                }
            }
            _ => {} // Done / OutputEnter / OutputLeave / Parent — no extra bookkeeping needed
        }
        if state.configured {
            state.draw();
        }
    }
}

// ---- foreign toplevel: ext list + handle -------------------------------------------------

impl Dispatch<ExtForeignToplevelListV1, ()> for Subject {
    fn event(state: &mut Self, _: &ExtForeignToplevelListV1, event: ext_foreign_toplevel_list_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                info!("ext foreign: new toplevel handle");
                state.ext_tops.push(ExtTop {
                    handle: toplevel,
                    identifier: String::new(),
                    title: String::new(),
                    app_id: String::new(),
                });
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                info!("ext foreign: list finished");
                state.ext_tops.clear();
                state.ext_list = None;
            }
            _ => {}
        }
        if state.configured {
            state.draw();
        }
    }
    event_created_child!(Subject, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Subject {
    fn event(state: &mut Self, handle: &ExtForeignToplevelHandleV1, event: ext_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                if let Some(t) = state.ext_top_mut(handle) {
                    t.identifier = identifier;
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(t) = state.ext_top_mut(handle) {
                    t.title = title;
                }
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(t) = state.ext_top_mut(handle) {
                    t.app_id = app_id;
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.ext_tops.retain(|t| &t.handle != handle);
                handle.destroy();
            }
            _ => {} // Done — nothing extra
        }
        if state.configured {
            state.draw();
        }
    }
}

impl Dispatch<WlBuffer, ()> for Subject {
    fn event(_: &mut Self, _: &WlBuffer, _: <WlBuffer as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
