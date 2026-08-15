//! The experimental window under test ("subject").
//!
//! It drives **raw** xdg-shell / xdg-decoration / viewporter / fractional-scale /
//! single-pixel-buffer objects (sctk's high-level `Window` auto-acks configures, which would
//! forbid the ack-abuse cases) while still using sctk for registry, shm `SlotPool`, output,
//! seat/pointer and (sub)compositor plumbing.
//!
//! It reads one [`Command`] per line on stdin (from the controller) and misbehaves on
//! demand. It renders a live state overlay and a crosshair at the current pointer location
//! on whichever of its surfaces the pointer is over.

use std::io::BufRead;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_pointer, delegate_registry, delegate_seat,
    delegate_shm, delegate_subcompositor,
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
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shm::{Shm, ShmHandler, slot::{Buffer as SlotBuffer, SlotPool}},
    subcompositor::SubcompositorState,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    globals::{GlobalList, registry_queue_init},
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_output::{self, WlOutput},
        wl_pointer::WlPointer,
        wl_seat::WlSeat,
        wl_shm::Format,
        wl_subsurface::WlSubsurface,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::{
    wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1,
        tearing_control::v1::client::{
            wp_tearing_control_manager_v1::WpTearingControlManagerV1,
            wp_tearing_control_v1::{PresentationHint, WpTearingControlV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    xdg::{
        decoration::zv1::client::{
            zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
            zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1},
        },
        shell::client::{
            xdg_popup::{self, XdgPopup},
            xdg_positioner::{self, XdgPositioner},
            xdg_surface::{self, XdgSurface},
            xdg_toplevel::{self, XdgToplevel},
            xdg_wm_base::{self, XdgWmBase},
        },
        toplevel_icon::v1::client::{
            xdg_toplevel_icon_manager_v1::{self, XdgToplevelIconManagerV1},
            xdg_toplevel_icon_v1::XdgToplevelIconV1,
        },
    },
};

use window_stress::canvas::{Canvas, color};
use window_stress::diag;
use window_stress::protocol::{Anchor, Command, DecoMode};
use window_stress::{font, info, warn};

// ----------------------------------------------------------------------------------------
// Dispatch userdata markers
// ----------------------------------------------------------------------------------------

/// Which of our surfaces an `xdg_surface` belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Main,
    Popup(u32),
}
/// Userdata for `xdg_surface` (carries the role).
#[derive(Clone, Copy, Debug)]
struct XdgSurfData(Role);
/// Userdata for `xdg_popup` (carries the popup id).
#[derive(Clone, Copy, Debug)]
struct PopupTag(u32);
/// Distinct userdata so our single-pixel `wl_buffer` does not collide with sctk's shm buffers.
#[derive(Clone, Copy, Debug)]
struct SpBuf;

// ----------------------------------------------------------------------------------------
// Scale behaviour
// ----------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScaleMode {
    Normal,
    FsHonor,
    FsIgnore,
    FsForce(u32), // numerator of n/120
    FsNoViewport,
    FsMismatch,
    DpiHonor,
    DpiIgnore,
    DpiForce(i32),
    DpiNondiv,
    DpiMismatch,
    DpiZero,
}

// ----------------------------------------------------------------------------------------
// Child surfaces
// ----------------------------------------------------------------------------------------

struct Sub {
    surface: WlSurface,
    subsurface: WlSubsurface,
    x: i32,
    y: i32,
    color: u32,
    #[allow(dead_code)]
    parent: Option<usize>,
}

/// A transient "you clicked here" marker shown on whichever surface received a button
/// press. It animates (an expanding ring) over [`CLICK_TTL`] frames, then clears — the same
/// diagnostic value as the pointer crosshair, but for *button* delivery: it confirms which
/// surface the compositor routed the click to, and at what local coordinate.
struct ClickFx {
    surface: WlSurface,
    x: f64,
    y: f64,
    ttl: u32,
    button: u32,
}

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

// ----------------------------------------------------------------------------------------
// Render resolution
// ----------------------------------------------------------------------------------------

struct Render {
    buf_w: i32,
    buf_h: i32,
    buffer_scale: i32,
    dest: Option<(i32, i32)>,
}
impl Render {
    fn new(buf_w: i32, buf_h: i32, buffer_scale: i32, dest: Option<(i32, i32)>) -> Self {
        Render { buf_w, buf_h, buffer_scale, dest }
    }
}

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
    subcompositor: SubcompositorState,
    qh: QueueHandle<Subject>,

    wm_base: XdgWmBase,
    #[allow(dead_code)]
    decoration_mgr: Option<ZxdgDecorationManagerV1>,
    viewporter: Option<WpViewporter>,
    #[allow(dead_code)]
    frac_mgr: Option<WpFractionalScaleManagerV1>,
    sp_mgr: Option<WpSinglePixelBufferManagerV1>,

    main_surface: WlSurface,
    main_xdg: XdgSurface,
    toplevel: XdgToplevel,
    decoration: Option<ZxdgToplevelDecorationV1>,
    viewport: Option<WpViewport>,
    _frac: Option<WpFractionalScaleV1>,
    pointer: Option<WlPointer>,

    mapped: bool,
    configured: bool,
    pending_size: (i32, i32),
    cur_size: (i32, i32),
    last_serial: Option<u32>,
    output_scale: i32,
    preferred_scale: u32, // x120; 0 = unknown

    deco_requested: DecoMode,
    deco_effective: Option<&'static str>,
    deco_ignore: bool,
    deco_badsize: bool,
    buf_delta: i32,
    geo_mismatch: bool,
    no_ack: bool,
    preack_once: bool,
    zero_once: bool,
    scale_mode: ScaleMode,
    vp_dest: Option<(i32, i32)>,
    vp_src: Option<(f64, f64, f64, f64)>,
    vp_animate: bool,
    anim_phase: i32,
    mapcycle: bool,
    mapcycle_tick: u32,
    /// `wp_tearing_control_v1` for the main surface (None if the compositor does
    /// not advertise the global).
    tearing_ctl: Option<WpTearingControlV1>,

    // --- xdg_toplevel_icon_v1 ---
    icon_mgr: Option<XdgToplevelIconManagerV1>,
    /// Sizes the compositor asked for at bind time (`icon_size` events).
    icon_sizes: Vec<i32>,
    /// Its own pool: icon buffers are never attached to a surface, so they are
    /// never released, and sharing the frame pool would let a redraw reuse the
    /// slot underneath a live icon.
    icon_pool: Option<SlotPool>,
    /// The icon currently set, plus the buffers it references. BOTH must outlive
    /// the icon being current — the compositor watches these buffers and raises
    /// `no_buffer` if one dies first.
    icon: Option<(XdgToplevelIconV1, Vec<SlotBuffer>)>,
    /// The previous generation, retired one change late: the `set_icon` for the
    /// new one has to reach the compositor's surface commit before the old
    /// object may go.
    icon_retired: Option<(XdgToplevelIconV1, Vec<SlotBuffer>)>,
    /// What was last requested, for the overlay.
    icon_label: String,
    /// Self-driven commit rate in Hz; 0 = default animation-only tick.
    commit_hz: u32,
    /// Draw the live FRAME/COMMIT counter into the overlay.
    show_counter: bool,
    /// Commits issued since the last report — the client-side half of the
    /// pacing measurement, so it can be compared against the compositor's
    /// presented-frame counter without trusting either alone.
    commits: u32,
    commits_since_report: std::time::Instant,

    // controller-mirrored popup positioner params
    pop_anchor: Anchor,
    pop_gravity: Anchor,
    pop_off: (i32, i32),
    pop_size: (i32, i32),
    sp_no_viewport: bool,

    subs: Vec<Sub>,
    popups: Vec<PopupEntry>,
    next_popup_id: u32,

    ptr: Option<(WlSurface, f64, f64)>,
    click: Option<ClickFx>,
    exit: bool,
}

const DEFAULT_W: i32 = 480;
const DEFAULT_H: i32 = 320;
const TITLE_NORMAL: i32 = 24;
const TITLE_BAD: i32 = 52;
const SUB_COLORS: [u32; 4] = [color::GREEN, color::MAGENTA, color::CYAN, color::YELLOW];
const POPUP_COLORS: [u32; 4] = [color::BLUE, color::RED, color::GREEN, color::MAGENTA];
/// Frames a click marker lives for (~16ms/frame from the tick timer).
const CLICK_TTL: u32 = 18;

/// Center-dot color encoding which button was pressed.
fn button_color(button: u32) -> u32 {
    match button {
        0x110 => color::RED,    // BTN_LEFT
        0x111 => color::GREEN,  // BTN_RIGHT
        0x112 => color::CYAN,   // BTN_MIDDLE
        _ => color::MAGENTA,
    }
}

/// Draw the click marker at buffer-pixel (cx, cy): a white-on-black expanding ring (visible on
/// any background) with a button-colored center dot. `ttl` counts down from [`CLICK_TTL`], so
/// the ring grows as the press ages.
fn click_marker(cv: &mut Canvas, cx: i32, cy: i32, ttl: u32, button: u32) {
    let r = 3 + (CLICK_TTL - ttl.min(CLICK_TTL)) as i32;
    cv.ring(cx, cy, r + 1, color::BLACK);
    cv.ring(cx, cy, r, color::WHITE);
    cv.rect(cx - 2, cy - 2, 5, 5, button_color(button));
    cv.frame(cx - 2, cy - 2, 5, 5, 1, color::BLACK);
}

fn main() {
    diag::set_role("subject");

    let args: Vec<String> = std::env::args().collect();
    let use_deco = !args.iter().any(|a| a == "--no-decoration");
    let use_vp = !args.iter().any(|a| a == "--no-viewporter");
    let use_fs = !args.iter().any(|a| a == "--no-fractional-scale");
    let use_sp = !args.iter().any(|a| a == "--no-single-pixel");

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

    let wm_base: XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");

    let decoration_mgr: Option<ZxdgDecorationManagerV1> =
        if use_deco { bind_opt(&globals, &qh, "zxdg_decoration_manager_v1") } else { None };
    let viewporter: Option<WpViewporter> =
        if use_vp { bind_opt(&globals, &qh, "wp_viewporter") } else { None };
    let frac_mgr: Option<WpFractionalScaleManagerV1> =
        if use_fs { bind_opt(&globals, &qh, "wp_fractional_scale_manager_v1") } else { None };
    let sp_mgr: Option<WpSinglePixelBufferManagerV1> =
        if use_sp { bind_opt(&globals, &qh, "wp_single_pixel_buffer_manager_v1") } else { None };
    let tearing_mgr: Option<WpTearingControlManagerV1> =
        bind_opt(&globals, &qh, "wp_tearing_control_manager_v1");
    // Always bound: an icon costs nothing until one of the `icon-*` commands
    // asks for it, and its absence is itself the first thing to check.
    let icon_mgr: Option<XdgToplevelIconManagerV1> =
        bind_opt(&globals, &qh, "xdg_toplevel_icon_manager_v1");
    if icon_mgr.is_none() {
        warn!("icon: compositor does not advertise xdg_toplevel_icon_manager_v1");
    }

    info!(
        "globals: xdg_wm_base + decoration={} viewporter={} fractional={} single_pixel={}",
        decoration_mgr.is_some(),
        viewporter.is_some(),
        frac_mgr.is_some(),
        sp_mgr.is_some()
    );

    let main_surface = compositor.create_surface(&qh);
    let main_xdg = wm_base.get_xdg_surface(&main_surface, &qh, XdgSurfData(Role::Main));
    let toplevel = main_xdg.get_toplevel(&qh, ());
    toplevel.set_title("y5 window-stress SUBJECT".into());
    toplevel.set_app_id("y5.window.stress.subject".into());

    let decoration =
        decoration_mgr.as_ref().map(|m| m.get_toplevel_decoration(&toplevel, &qh, ()));
    let viewport = viewporter.as_ref().map(|v| v.get_viewport(&main_surface, &qh, ()));
    let frac = frac_mgr.as_ref().map(|m| m.get_fractional_scale(&main_surface, &qh, ()));
    // Created up front but left at the protocol default (vsync) until asked —
    // binding must not itself change behaviour.
    let tearing_ctl = tearing_mgr.as_ref().map(|m| m.get_tearing_control(&main_surface, &qh, ()));
    info!("tearing_control global: {}", tearing_mgr.is_some());

    main_surface.commit();

    let mut subject = Subject {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        subcompositor,
        qh: qh.clone(),
        wm_base,
        decoration_mgr,
        viewporter,
        frac_mgr,
        sp_mgr,
        main_surface,
        main_xdg,
        toplevel,
        decoration,
        viewport,
        _frac: frac,
        pointer: None,
        mapped: false,
        configured: false,
        pending_size: (DEFAULT_W, DEFAULT_H),
        cur_size: (DEFAULT_W, DEFAULT_H),
        last_serial: None,
        output_scale: 1,
        preferred_scale: 0,
        deco_requested: DecoMode::Server,
        deco_effective: None,
        deco_ignore: false,
        deco_badsize: false,
        buf_delta: 0,
        geo_mismatch: false,
        no_ack: false,
        preack_once: false,
        zero_once: false,
        scale_mode: ScaleMode::Normal,
        vp_dest: None,
        vp_src: None,
        vp_animate: false,
        anim_phase: 0,
        mapcycle: false,
        mapcycle_tick: 0,
        tearing_ctl,
        icon_mgr,
        icon_sizes: Vec::new(),
        icon_pool: None,
        icon: None,
        icon_retired: None,
        icon_label: String::from("none"),
        commit_hz: 0,
        show_counter: true,
        commits: 0,
        commits_since_report: std::time::Instant::now(),
        pop_anchor: Anchor::BottomLeft,
        pop_gravity: Anchor::BottomRight,
        pop_off: (0, 0),
        pop_size: (160, 120),
        sp_no_viewport: false,
        subs: Vec::new(),
        popups: Vec::new(),
        next_popup_id: 1,
        ptr: None,
        click: None,
        exit: false,
    };

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
            // Controller closed the pipe (e.g. it exited): shut down too.
            ChanEvent::Closed => {
                info!("stdin closed; exiting");
                state.exit = true;
            }
        })
        .expect("insert stdin channel");

    // ~60Hz timer drives animations (vp-animate) and the map/unmap cycle.
    event_loop
        .handle()
        .insert_source(
            Timer::from_duration(std::time::Duration::from_millis(16)),
            |_, _, state: &mut Subject| {
                state.tick();
                TimeoutAction::ToDuration(state.tick_interval())
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

/// Bind an optional global by name, logging absence.
fn bind_opt<I>(globals: &GlobalList, qh: &QueueHandle<Subject>, name: &str) -> Option<I>
where
    I: Proxy + 'static,
    Subject: Dispatch<I, ()>,
{
    match globals.bind::<I, Subject, ()>(qh, 1..=1, ()) {
        Ok(p) => Some(p),
        Err(e) => {
            warn!("global {name} unavailable: {e}");
            None
        }
    }
}

impl Subject {
    // ---- command handling ------------------------------------------------------------

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
        // Commands that manage their own surface commit must NOT be clobbered by a trailing
        // redraw of the main surface.
        let mut redraw = true;
        match cmd {
            Command::DecoMode(m) => {
                self.deco_requested = m;
                if let Some(d) = &self.decoration {
                    match m {
                        DecoMode::Server => d.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide),
                        DecoMode::Client => d.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide),
                        DecoMode::None => d.unset_mode(),
                    }
                } else {
                    warn!("no decoration manager; mode request ignored");
                }
            }
            Command::DecoIgnore => self.deco_ignore = !self.deco_ignore,
            Command::DecoBadsize => self.deco_badsize = !self.deco_badsize,

            Command::BufAgreed => self.buf_delta = 0,
            Command::BufDelta(d) => self.buf_delta = d,
            Command::BufZero => self.zero_once = true,
            Command::BufNoack => self.no_ack = !self.no_ack,
            Command::BufPreack => self.preack_once = true,
            Command::GeoMismatch => self.geo_mismatch = !self.geo_mismatch,

            Command::PopupAdd => self.add_popup(None),
            Command::PopupNest => {
                let parent = self.popups.last().map(|p| p.id);
                self.add_popup(parent);
            }
            Command::PopupAnchor(a) => self.pop_anchor = a,
            Command::PopupGravity(g) => self.pop_gravity = g,
            Command::PopupOff(x, y) => self.pop_off = (x, y),
            Command::PopupSize(w, h) => self.pop_size = (w as i32, h as i32),
            Command::PopupMove(dx, dy) => self.reposition_last_popup(dx, dy),
            Command::PopupClose => {
                if let Some(p) = self.popups.pop() {
                    p.popup.destroy();
                    p.xdg_surface.destroy();
                    p.surface.destroy();
                }
            }

            Command::SubAdd => self.add_sub(None),
            Command::SubNest => {
                let parent = self.subs.len().checked_sub(1);
                self.add_sub(parent);
            }
            Command::SubMove(dx, dy) => {
                if let Some(s) = self.subs.last_mut() {
                    s.x += dx;
                    s.y += dy;
                    s.subsurface.set_position(s.x, s.y);
                    s.surface.commit();
                }
                self.main_surface.commit();
            }
            Command::SubSync => {
                if let Some(s) = self.subs.last() {
                    s.subsurface.set_sync();
                }
            }
            Command::SubDesync => {
                if let Some(s) = self.subs.last() {
                    s.subsurface.set_desync();
                }
            }
            Command::SubRemove => {
                if let Some(s) = self.subs.pop() {
                    s.subsurface.destroy();
                    s.surface.destroy();
                    self.main_surface.commit();
                }
            }

            Command::VpDest(w, h) => self.vp_dest = Some((w, h)),
            Command::VpDestDelta(d) => {
                let base = self.vp_dest.unwrap_or(self.cur_size);
                self.vp_dest = Some((base.0 + d, base.1 + d));
            }
            Command::VpSrc(x, y, w, h) => self.vp_src = Some((x, y, w, h)),
            Command::VpAnimate(on) => self.vp_animate = on,
            Command::VpUnset => {
                self.vp_dest = None;
                self.vp_src = None;
            }
            Command::VpBad => {
                redraw = false;
                if let Some(v) = &self.viewport {
                    warn!("vp-bad: out-of-bounds source (expect compositor protocol error)");
                    v.set_source(0.0, 0.0, 100000.0, 100000.0);
                    self.main_surface.commit();
                } else {
                    warn!("no viewport");
                }
            }

            Command::FsHonor => self.scale_mode = ScaleMode::FsHonor,
            Command::FsIgnore => self.scale_mode = ScaleMode::FsIgnore,
            Command::FsScale(n) => self.scale_mode = ScaleMode::FsForce(n),
            Command::FsNoViewport => self.scale_mode = ScaleMode::FsNoViewport,
            Command::FsMismatch => self.scale_mode = ScaleMode::FsMismatch,

            Command::DpiHonor => self.scale_mode = ScaleMode::DpiHonor,
            Command::DpiIgnore => self.scale_mode = ScaleMode::DpiIgnore,
            Command::DpiScale(n) => self.scale_mode = ScaleMode::DpiForce(n),
            Command::DpiNondiv => self.scale_mode = ScaleMode::DpiNondiv,
            Command::DpiMismatch => self.scale_mode = ScaleMode::DpiMismatch,
            Command::DpiZero => self.scale_mode = ScaleMode::DpiZero,

            Command::SpFill(r, g, b, a) => {
                redraw = false;
                self.single_pixel_main(r, g, b, a);
            }
            Command::SpSub(r, g, b, a) => self.single_pixel_sub(r, g, b, a),
            Command::SpNoViewport => self.sp_no_viewport = !self.sp_no_viewport,

            Command::IconName(name) => self.set_icon(Some(&name), &[]),
            Command::IconBuffer(edges) => self.set_icon(None, &edges),
            Command::IconBoth(name) => self.set_icon(Some(&name), &[64]),
            Command::IconClear => self.clear_icon(),
            Command::Map => self.set_mapped(true),
            Command::Unmap => self.set_mapped(false),
            Command::MapCycle(on) => self.mapcycle = on,
            Command::Tearing(on) => match &self.tearing_ctl {
                Some(ctl) => {
                    ctl.set_presentation_hint(if on {
                        PresentationHint::Async
                    } else {
                        PresentationHint::Vsync
                    });
                    // The hint is double-buffered surface state: it takes effect
                    // on the next commit, not on the request.
                    self.main_surface.commit();
                    info!("tearing hint = {}", if on { "async" } else { "vsync" });
                }
                None => warn!("tearing: compositor does not advertise wp_tearing_control_manager_v1"),
            },
            Command::ShowCounter(on) => {
                self.show_counter = on;
                info!("live counter overlay = {on}");
                if self.configured && self.mapped {
                    self.draw_main();
                }
            }
            Command::CommitRate(hz) => {
                self.commit_hz = hz;
                self.commits = 0;
                self.commits_since_report = std::time::Instant::now();
                info!("commit rate = {hz} Hz (0 = animation tick only)");
            }
            Command::Size(w, h) => {
                self.pending_size = (w as i32, h as i32);
                self.cur_size = (w as i32, h as i32);
            }
            Command::Quit => {
                redraw = false;
                self.exit = true;
            }
        }
        if redraw && self.configured && self.mapped {
            self.draw_main();
        }
    }

    // ---- lifecycle ------------------------------------------------------------------

    // ---------------------------------------------------------------- icon --

    /// Publish an `xdg_toplevel_icon_v1` for this toplevel: a stock icon NAME, a
    /// set of pixel BUFFERS, or both on the same icon object.
    ///
    /// Each buffer is a solid square colour-coded BY EDGE SIZE (see [`icon_tint`])
    /// so which size the compositor picked is visible in whatever it draws, plus a
    /// half-alpha quadrant — premultiplied, as `wl_shm` requires — so a consumer
    /// that forgets to un-premultiply shows a visibly darker quarter.
    fn set_icon(&mut self, name: Option<&str>, edges: &[i32]) {
        let Some(mgr) = self.icon_mgr.clone() else {
            warn!("icon: compositor does not advertise xdg_toplevel_icon_manager_v1");
            return;
        };
        let icon = mgr.create_icon(&self.qh, ());
        if let Some(name) = name {
            icon.set_name(name.to_string());
        }
        let mut buffers = Vec::new();
        for (i, edge) in edges.iter().copied().enumerate() {
            match self.icon_buffer(edge, icon_tint(i)) {
                Some(buffer) => {
                    // scale 1: these are the sizes we claim, not logical ones.
                    icon.add_buffer(buffer.wl_buffer(), 1);
                    buffers.push(buffer);
                }
                None => warn!("icon: could not allocate a {edge}x{edge} buffer"),
            }
        }
        mgr.set_icon(&self.toplevel, Some(&icon));
        // The icon is double-buffered surface state — it lands on commit, and the
        // retire below must not overtake that.
        self.main_surface.commit();
        self.retire_icon();
        self.icon_retired = self.icon.take();
        self.icon = Some((icon, buffers));
        self.icon_label = match (name, edges.is_empty()) {
            (Some(n), true) => format!("name {n}"),
            (Some(n), false) => format!("name {n} + {} buf", edges.len()),
            (None, _) => format!("{} buf {:?}", edges.len(), edges),
        };
        info!("icon: set ({})", self.icon_label);
        self.redraw_icon_overlay();
    }

    /// `set_icon(null)` — the toplevel declares no icon at all, which is the case
    /// a compositor should answer from its own inference instead.
    fn clear_icon(&mut self) {
        let Some(mgr) = self.icon_mgr.clone() else {
            warn!("icon: compositor does not advertise xdg_toplevel_icon_manager_v1");
            return;
        };
        mgr.set_icon(&self.toplevel, None);
        self.main_surface.commit();
        self.retire_icon();
        self.icon_retired = self.icon.take();
        self.icon_label = String::from("none");
        info!("icon: cleared");
        self.redraw_icon_overlay();
    }

    /// Repaint so the overlay's icon line reflects the change.
    fn redraw_icon_overlay(&mut self) {
        if self.configured && self.mapped {
            self.draw_main();
        }
    }

    /// Drop the generation before last. Destroying the icon object is what
    /// releases the compositor's watch on its buffers, so the buffers go after it
    /// — never before, or the compositor raises `no_buffer` on us.
    fn retire_icon(&mut self) {
        let Some((icon, buffers)) = self.icon_retired.take() else { return };
        icon.destroy();
        drop(buffers);
    }

    /// One square icon buffer. Its own pool, grown on demand.
    fn icon_buffer(&mut self, edge: i32, tint: u32) -> Option<SlotBuffer> {
        if edge <= 0 {
            return None;
        }
        let need = (edge * edge * 4) as usize;
        let pool = match &mut self.icon_pool {
            Some(pool) => pool,
            None => {
                self.icon_pool = SlotPool::new(need.max(256 * 256 * 4), &self.shm).ok();
                self.icon_pool.as_mut()?
            }
        };
        let (buffer, slice) = pool.create_buffer(edge, edge, edge * 4, Format::Argb8888).ok()?;
        let mut cv = Canvas::new(slice, edge, edge);
        // Fully transparent margin proves the alpha channel survives the trip.
        cv.clear(0x0000_0000);
        let inset = (edge / 8).max(1);
        cv.rect(inset, inset, edge - 2 * inset, edge - 2 * inset, tint);
        cv.frame(inset, inset, edge - 2 * inset, edge - 2 * inset, (edge / 16).max(1), 0xFF10_1418);
        // Half-alpha quadrant, PREMULTIPLIED (each channel already scaled by a).
        let half = edge / 2;
        cv.rect(half, half, edge - half - inset, edge - half - inset, premultiply(tint, 128));
        Some(buffer)
    }

    fn set_mapped(&mut self, on: bool) {
        if on && !self.mapped {
            self.configured = false;
            self.main_surface.commit();
            self.mapped = true;
        } else if !on && self.mapped {
            self.main_surface.attach(None, 0, 0);
            self.main_surface.commit();
            self.mapped = false;
        }
    }

    /// The tick period: the self-driven commit rate when set, else the default
    /// ~60Hz animation cadence.
    fn tick_interval(&self) -> std::time::Duration {
        match self.commit_hz {
            0 => std::time::Duration::from_millis(16),
            hz => std::time::Duration::from_micros(1_000_000 / hz.max(1) as u64),
        }
    }

    /// Commit on our OWN clock, never on a frame callback. That independence is
    /// what makes the compositor's flip rate a measurement rather than an echo
    /// of whatever cadence it already chose to hand us.
    fn drive_commit(&mut self) {
        if self.commit_hz == 0 || !self.configured || !self.mapped {
            return;
        }
        // MUST go through draw_main: it attaches a FRESH buffer. Damaging and
        // committing the existing one leaves the surface content identical, and
        // the compositor's damage tracker then elides the frame entirely — no
        // composite, no flip, and the commit rate has no observable effect at
        // any value. The drawn overlay carries `commits`, so every frame really
        // does differ and the tearing is visible rather than merely counted.
        // (`commits` is incremented inside `draw_main`, so it counts EVERY
        // main-surface commit rather than only the paced ones.)
        self.draw_main();
        let elapsed = self.commits_since_report.elapsed();
        if elapsed >= std::time::Duration::from_secs(1) {
            info!(
                "commits: {:.1}/s (requested {} Hz)",
                self.commits as f32 / elapsed.as_secs_f32(),
                self.commit_hz
            );
            self.commits = 0;
            self.commits_since_report = std::time::Instant::now();
        }
    }

    fn tick(&mut self) {
        self.drive_commit();
        if self.vp_animate && self.configured && self.mapped {
            self.anim_phase = (self.anim_phase + 4) % 400;
            self.draw_main();
        }
        // Age the click marker one frame and redraw its surface (the final frame clears it).
        let expiring = match &mut self.click {
            Some(fx) if fx.ttl > 0 => {
                fx.ttl -= 1;
                Some((fx.surface.clone(), false))
            }
            Some(fx) => Some((fx.surface.clone(), true)),
            None => None,
        };
        if let Some((surface, done)) = expiring {
            if done {
                self.click = None;
            }
            self.redraw_for(&surface);
        }
        if self.mapcycle {
            self.mapcycle_tick += 1;
            if self.mapcycle_tick >= 60 {
                self.mapcycle_tick = 0;
                let want = !self.mapped;
                self.set_mapped(want);
            }
        }
    }

    /// Redraw whichever of our surfaces `surface` is (main, a subsurface, or a popup).
    fn redraw_for(&mut self, surface: &WlSurface) {
        if surface == &self.main_surface {
            if self.configured && self.mapped {
                self.draw_main();
            }
        } else if let Some(i) = self.subs.iter().position(|s| &s.surface == surface) {
            self.draw_sub(i);
        } else if let Some(i) = self.popups.iter().position(|p| &p.surface == surface) {
            if self.popups[i].configured {
                self.draw_popup(i);
            }
        }
    }

    // ---- popups ---------------------------------------------------------------------

    fn add_popup(&mut self, parent_id: Option<u32>) {
        let qh = self.qh.clone();
        let id = self.next_popup_id;
        self.next_popup_id += 1;

        let (pw, ph) = self.pop_size;
        let (lw, lh) = self.cur_size;

        let positioner = self.wm_base.create_positioner(&qh, ());
        positioner.set_size(pw.max(1), ph.max(1));
        positioner.set_anchor_rect(0, 0, lw.max(1), lh.max(1));
        positioner.set_anchor(to_anchor(self.pop_anchor));
        positioner.set_gravity(to_gravity(self.pop_gravity));
        positioner.set_offset(self.pop_off.0, self.pop_off.1);
        positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::empty());

        let parent_xdg = match parent_id.and_then(|pid| self.popups.iter().find(|p| p.id == pid)) {
            Some(p) => p.xdg_surface.clone(),
            None => self.main_xdg.clone(),
        };

        let surface = self.compositor.create_surface(&qh);
        let xdg_surface =
            self.wm_base.get_xdg_surface(&surface, &qh, XdgSurfData(Role::Popup(id)));
        let popup = xdg_surface.get_popup(Some(&parent_xdg), &positioner, &qh, PopupTag(id));
        positioner.destroy();
        surface.commit();

        info!(
            "popup {id} anchor={:?} gravity={:?} off={:?} size={pw}x{ph} parent={:?}",
            self.pop_anchor, self.pop_gravity, self.pop_off, parent_id
        );

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
        let (lw, lh) = self.cur_size;
        if let Some(p) = self.popups.last_mut() {
            p.off = (p.off.0 + dx, p.off.1 + dy);
            if p.popup.version() >= 3 {
                let positioner = self.wm_base.create_positioner(&qh, ());
                positioner.set_size(p.w.max(1), p.h.max(1));
                positioner.set_anchor_rect(0, 0, lw.max(1), lh.max(1));
                positioner.set_anchor(to_anchor(p.anchor));
                positioner.set_gravity(to_gravity(p.gravity));
                positioner.set_offset(p.off.0, p.off.1);
                positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::empty());
                p.popup.reposition(&positioner, p.id);
                positioner.destroy();
                info!("popup {} reposition off={:?}", p.id, p.off);
            } else {
                warn!("popup reposition needs xdg v3; have v{}", p.popup.version());
            }
        }
    }

    // ---- subsurfaces ----------------------------------------------------------------

    fn add_sub(&mut self, parent_idx: Option<usize>) {
        let qh = self.qh.clone();
        let parent_surface = match parent_idx {
            Some(i) => self.subs[i].surface.clone(),
            None => self.main_surface.clone(),
        };
        let (subsurface, surface) = self.subcompositor.create_subsurface(parent_surface, &qh);
        let n = self.subs.len() as i32;
        let x = 40 + n * 24;
        let y = 60 + n * 24;
        subsurface.set_position(x, y);
        subsurface.set_desync();
        let color = SUB_COLORS[self.subs.len() % SUB_COLORS.len()];
        self.subs.push(Sub { surface, subsurface, x, y, color, parent: parent_idx });
        let idx = self.subs.len() - 1;
        self.draw_sub(idx);
        self.main_surface.commit();
        info!("subsurface {idx} at ({x},{y}) parent={:?}", parent_idx);
    }

    // ---- single pixel ---------------------------------------------------------------

    fn single_pixel_main(&mut self, r: u8, g: u8, b: u8, a: u8) {
        let qh = self.qh.clone();
        let Some(mgr) = &self.sp_mgr else {
            warn!("no single-pixel-buffer manager");
            return;
        };
        let (lw, lh) = self.cur_size;
        let buffer =
            mgr.create_u32_rgba_buffer(scale8(r), scale8(g), scale8(b), scale8(a), &qh, SpBuf);
        self.main_surface.attach(Some(&buffer), 0, 0);
        if let (false, Some(v)) = (self.sp_no_viewport, &self.viewport) {
            v.set_source(-1.0, -1.0, -1.0, -1.0);
            v.set_destination(lw.max(1), lh.max(1));
        } else {
            warn!("sp-fill without viewport: window will be a literal 1x1 buffer");
        }
        self.main_surface.set_buffer_scale(1);
        self.main_surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        if !self.no_ack {
            if let Some(s) = self.last_serial.take() {
                self.main_xdg.ack_configure(s);
            }
        }
        self.main_surface.commit();
        info!("sp-fill rgba=({r},{g},{b},{a}) viewport_dest={lw}x{lh}");
    }

    fn single_pixel_sub(&mut self, r: u8, g: u8, b: u8, a: u8) {
        let qh = self.qh.clone();
        let Some(mgr) = &self.sp_mgr else {
            warn!("no single-pixel-buffer manager");
            return;
        };
        let (subsurface, surface) =
            self.subcompositor.create_subsurface(self.main_surface.clone(), &qh);
        let n = self.subs.len() as i32;
        let x = 60 + n * 24;
        let y = 90 + n * 24;
        subsurface.set_position(x, y);
        subsurface.set_desync();
        let buffer =
            mgr.create_u32_rgba_buffer(scale8(r), scale8(g), scale8(b), scale8(a), &qh, SpBuf);
        surface.attach(Some(&buffer), 0, 0);
        if let Some(vp) = &self.viewporter {
            let v = vp.get_viewport(&surface, &qh, ());
            v.set_destination(120, 80);
        }
        surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        surface.commit();
        self.main_surface.commit();
        self.subs.push(Sub {
            surface,
            subsurface,
            x,
            y,
            color: 0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
            parent: None,
        });
        info!("sp-sub rgba=({r},{g},{b},{a}) at ({x},{y})");
    }

    // ---- drawing --------------------------------------------------------------------

    fn draw_main(&mut self) {
        if !self.configured || !self.mapped {
            return;
        }
        let (lw, lh) = (self.cur_size.0.max(1), self.cur_size.1.max(1));
        let render = self.resolve_scale(lw, lh);
        let bw = (render.buf_w + self.buf_delta).max(1);
        let bh = (render.buf_h + self.buf_delta).max(1);

        let csd = self.csd_drawn();
        let title_h = if csd {
            if self.deco_badsize { TITLE_BAD } else { TITLE_NORMAL }
        } else {
            0
        };

        if self.geo_mismatch {
            self.main_xdg.set_window_geometry(17, 17, (lw - 60).max(1), (lh - 60).max(1));
        } else {
            self.main_xdg.set_window_geometry(0, 0, lw, lh);
        }

        // Zero-buffer one-shot: attach a null buffer and bail.
        if self.zero_once {
            self.zero_once = false;
            self.main_surface.attach(None, 0, 0);
            self.ack_now();
            self.main_surface.commit();
            warn!("buf-zero: attached null buffer");
            return;
        }

        // Snapshot everything the painter needs BEFORE borrowing the pool mutably.
        let mut dest = render.dest.or(self.vp_dest);
        // vp-animate: oscillate the viewport destination ~60Hz to stress relayout. The
        // buffer pixels stay fixed; only the logical (destination) size pulses, so the
        // compositor must re-lay-out the surface (and any children) every frame.
        if self.vp_animate && self.viewport.is_some() {
            let amp = (self.anim_phase - 200).abs(); // triangle wave 0..200..0
            let (base_w, base_h) = dest.unwrap_or((lw, lh));
            dest = Some((base_w + amp, base_h + amp));
        }
        let buffer_scale = render.buffer_scale;
        let badsize = self.deco_badsize;
        let ptr_main = self
            .ptr
            .as_ref()
            .filter(|(s, _, _)| s == &self.main_surface)
            .map(|(_, x, y)| (*x, *y));
        let click_main = self
            .click
            .as_ref()
            .filter(|fx| fx.surface == self.main_surface)
            .map(|fx| (fx.x, fx.y, fx.ttl, fx.button));
        let overlay = self.overlay_lines(lw, lh, bw, bh, &render, dest);
        let sx = bw as f32 / lw as f32;
        let sy = bh as f32 / lh as f32;

        let stride = bw * 4;
        let (buffer, slice) =
            self.pool.create_buffer(bw, bh, stride, Format::Argb8888).expect("create buffer");
        {
            let mut cv = Canvas::new(slice, bw, bh);
            cv.clear(0xFF101418);
            cv.frame(0, 0, bw, bh, 2, color::CYAN);
            if title_h > 0 {
                let th = (title_h as f32 * sy) as i32;
                cv.rect(0, 0, bw, th, 0xFF2A3550);
                font::text(&mut cv, 6, 6, 2, color::WHITE, "CSD TITLEBAR");
                if badsize {
                    cv.frame(0, 0, bw, th, 1, color::RED);
                }
            }
            let mut yy = title_h + 8;
            for s in &overlay {
                font::text(&mut cv, 8, yy, 2, color::LTGREY, s);
                yy += 20;
            }
            // The compositor delivers pointer/button coords in SURFACE-LOGICAL space
            // (`view.dst`): the viewport destination if one is set, else buffer_px /
            // buffer_scale. This canvas is `bw x bh` *buffer pixels*, so map logical -> buffer
            // pixels; otherwise markers land at the wrong place whenever buffer_scale or the
            // viewport make logical != buffer (DPI/FRAC/VP). If a marker still doesn't sit under
            // the real cursor after this mapping, the compositor's delivery is wrong.
            let to_buf = |px: f64, py: f64| {
                let (vw, vh) = match dest {
                    Some((dw, dh)) => (dw.max(1) as f32, dh.max(1) as f32),
                    None => {
                        let bs = buffer_scale.max(1) as f32;
                        (bw as f32 / bs, bh as f32 / bs)
                    }
                };
                ((px as f32 * bw as f32 / vw) as i32, (py as f32 * bh as f32 / vh) as i32)
            };
            if let Some((px, py)) = ptr_main {
                let (bx, by) = to_buf(px, py);
                cv.crosshair(bx, by, 12, color::YELLOW);
                font::text(&mut cv, bx + 8, by + 8, 2, color::YELLOW, &format!("{px:.0},{py:.0}"));
            }
            if let Some((px, py, ttl, button)) = click_main {
                let (bx, by) = to_buf(px, py);
                click_marker(&mut cv, bx, by, ttl, button);
            }
        }

        self.main_surface.set_buffer_scale(buffer_scale);
        if let Some(v) = &self.viewport {
            match dest {
                Some((dw, dh)) => v.set_destination(dw.max(1), dh.max(1)),
                None => v.set_destination(-1, -1),
            }
            match self.vp_src {
                Some((x, y, w, h)) => v.set_source(x, y, w, h),
                None => v.set_source(-1.0, -1.0, -1.0, -1.0),
            }
        }

        let preack = self.preack_once;
        self.preack_once = false;
        if !preack {
            self.ack_now();
        }
        self.main_surface.damage_buffer(0, 0, bw, bh);
        buffer.attach_to(&self.main_surface).expect("attach");
        self.main_surface.commit();
        // Counted HERE, not at the paced tick: `draw_main` is also reached from
        // vp-animate, pointer/click events and configure, each of which attaches
        // and commits too. Counting ticks would undercount those and make the
        // overlay disagree with the compositor's frame rate for reasons that
        // have nothing to do with the compositor — VP ANIMATE alone would look
        // like a 2x compositor bug.
        self.commits += 1;
        if preack {
            self.ack_now();
            warn!("buf-preack: committed buffer before ack_configure");
        }
    }

    fn overlay_lines(
        &self,
        lw: i32,
        lh: i32,
        bw: i32,
        bh: i32,
        render: &Render,
        dest: Option<(i32, i32)>,
    ) -> Vec<String> {
        // The counter line is what makes each frame's pixels DIFFER, so it is
        // also what makes a tear visible (you see the number split across the
        // seam). With it off, successive frames can be pixel-identical: the
        // compositor still composites and flips — draw_main attaches a fresh
        // buffer with explicit full damage either way — but the tear has nothing
        // to reveal, so it is the control case for "is that seam real".
        let mut lines = vec![
            format!("CONFIGURE {lw}x{lh}"),
            format!("BUFFER {bw}x{bh}  SCALE {}", render.buffer_scale),
            format!("VP DEST {}", dest.map(|(w, h)| format!("{w}x{h}")).unwrap_or("NONE".into())),
            format!("OUT SCALE {}  FRAC {}", self.output_scale, frac_str(self.preferred_scale)),
            format!("SCALEMODE {:?}", self.scale_mode),
            format!(
                "DECO req={:?} eff={} ign={}",
                self.deco_requested,
                self.deco_effective.unwrap_or("?"),
                self.deco_ignore
            ),
            format!(
                "ACK {}  GEOMIS {}  BUFDELTA {}",
                if self.no_ack { "OFF" } else { "ON" },
                self.geo_mismatch,
                self.buf_delta
            ),
            format!("SUBS {}  POPUPS {}", self.subs.len(), self.popups.len()),
            format!(
                "ICON {}  [mgr {} sizes {:?}]",
                self.icon_label,
                if self.icon_mgr.is_some() { "yes" } else { "NO" },
                self.icon_sizes
            ),
        ];
        if self.show_counter {
            lines.insert(0, format!("FRAME {}  COMMIT {} Hz", self.commits, self.commit_hz));
        }
        lines
    }

    fn ack_now(&mut self) {
        if self.no_ack {
            return;
        }
        if let Some(s) = self.last_serial.take() {
            self.main_xdg.ack_configure(s);
        }
    }

    /// Whether we are drawing our own (client-side) decorations.
    fn csd_drawn(&self) -> bool {
        if self.deco_ignore {
            return true;
        }
        match self.deco_effective {
            Some("client") => true,
            Some("server") => false,
            _ => matches!(self.deco_requested, DecoMode::Client | DecoMode::None),
        }
    }

    fn draw_sub(&mut self, idx: usize) {
        let (w, h) = (140i32, 100i32);
        let stride = w * 4;
        let sub_color = self.subs[idx].color;
        let surface = self.subs[idx].surface.clone();
        let ptr_local = self
            .ptr
            .as_ref()
            .filter(|(s, _, _)| s == &surface)
            .map(|(_, x, y)| (*x, *y));
        let click_local = self
            .click
            .as_ref()
            .filter(|fx| fx.surface == surface)
            .map(|fx| (fx.x, fx.y, fx.ttl, fx.button));
        let (buffer, slice) =
            self.pool.create_buffer(w, h, stride, Format::Argb8888).expect("sub buffer");
        {
            let mut cv = Canvas::new(slice, w, h);
            cv.clear(sub_color);
            cv.frame(0, 0, w, h, 2, color::WHITE);
            font::text(&mut cv, 6, 6, 2, color::BLACK, &format!("SUB {idx}"));
            if let Some((px, py)) = ptr_local {
                cv.crosshair(px as i32, py as i32, 10, color::BLACK);
                font::text(&mut cv, px as i32 + 8, py as i32 + 8, 1, color::BLACK, &format!("{px:.0},{py:.0}"));
            }
            if let Some((px, py, ttl, button)) = click_local {
                click_marker(&mut cv, px as i32, py as i32, ttl, button);
            }
        }
        surface.damage_buffer(0, 0, w, h);
        buffer.attach_to(&surface).expect("attach sub");
        surface.commit();
    }

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
        let click_local = self
            .click
            .as_ref()
            .filter(|fx| fx.surface == surface)
            .map(|fx| (fx.x, fx.y, fx.ttl, fx.button));
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
                font::text(&mut cv, px as i32 + 8, py as i32 + 8, 1, color::BLACK, &format!("{px:.0},{py:.0}"));
            }
            if let Some((px, py, ttl, button)) = click_local {
                click_marker(&mut cv, px as i32, py as i32, ttl, button);
            }
        }
        if let Some(s) = serial {
            self.popups[idx].xdg_surface.ack_configure(s);
        }
        surface.damage_buffer(0, 0, w, h);
        buffer.attach_to(&surface).expect("attach popup");
        surface.commit();
    }

    /// Compute buffer pixel size + declared buffer scale + implicit viewport destination
    /// for the current [`ScaleMode`].
    fn resolve_scale(&self, lw: i32, lh: i32) -> Render {
        let os = self.output_scale.max(1);
        let frac =
            if self.preferred_scale > 0 { self.preferred_scale as f32 / 120.0 } else { 1.0 };
        let r = |v: f32| v.round().max(1.0) as i32;
        match self.scale_mode {
            ScaleMode::Normal => Render::new(lw, lh, 1, None),
            ScaleMode::FsHonor => {
                Render::new(r(lw as f32 * frac), r(lh as f32 * frac), 1, Some((lw, lh)))
            }
            ScaleMode::FsIgnore => Render::new(lw, lh, 1, Some((lw, lh))),
            ScaleMode::FsForce(n) => {
                let f = n as f32 / 120.0;
                Render::new(r(lw as f32 * f), r(lh as f32 * f), 1, Some((lw, lh)))
            }
            ScaleMode::FsNoViewport => Render::new(r(lw as f32 * frac), r(lh as f32 * frac), 1, None),
            ScaleMode::FsMismatch => {
                Render::new(r(lw as f32 * frac), r(lh as f32 * frac), 1, Some((lw * 2, lh * 2)))
            }
            ScaleMode::DpiHonor => Render::new(lw * os, lh * os, os, None),
            ScaleMode::DpiIgnore => Render::new(lw, lh, 1, None),
            ScaleMode::DpiForce(n) => {
                let s = n.max(1);
                Render::new(lw * s, lh * s, s, None)
            }
            ScaleMode::DpiNondiv => {
                let s = os.max(2);
                Render::new(lw * s + 1, lh * s + 1, s, None)
            }
            ScaleMode::DpiMismatch => Render::new(lw, lh, os + 1, None),
            ScaleMode::DpiZero => Render::new(lw, lh, 0, None),
        }
    }
}

fn frac_str(x120: u32) -> String {
    if x120 == 0 { "?".into() } else { format!("{:.3}", x120 as f32 / 120.0) }
}

/// Expand an 8-bit channel to the 32-bit value the single-pixel protocol wants.
fn scale8(v: u8) -> u32 {
    (v as u32) * 0x0101_0101
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

impl CompositorHandler for Subject {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, new: i32) {
        if surface == &self.main_surface {
            self.output_scale = new.max(1);
            self.draw_main();
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        if surface == &self.main_surface {
            self.draw_main();
        } else if let Some(i) = self.subs.iter().position(|s| &s.surface == surface) {
            self.draw_sub(i);
        } else if let Some(i) = self.popups.iter().position(|p| &p.surface == surface) {
            self.draw_popup(i);
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
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
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, cap: Capability) {
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
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
        if self.configured && self.mapped {
            self.draw_main();
            for i in 0..self.subs.len() {
                self.draw_sub(i);
            }
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
delegate_registry!(Subject);

// ==========================================================================================
// Manual Dispatch impls for raw protocol objects
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
            match data.0 {
                Role::Main => {
                    state.cur_size = state.pending_size;
                    state.last_serial = Some(serial);
                    state.configured = true;
                    state.mapped = true;
                    state.draw_main();
                }
                Role::Popup(id) => {
                    if let Some(idx) = state.popups.iter().position(|p| p.id == id) {
                        state.popups[idx].serial = Some(serial);
                        state.popups[idx].configured = true;
                        state.draw_popup(idx);
                    }
                }
            }
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Subject {
    fn event(state: &mut Self, _: &XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 && height > 0 {
                    state.pending_size = (width, height);
                }
            }
            xdg_toplevel::Event::Close => {
                info!("toplevel close requested");
                state.exit = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgPopup, PopupTag> for Subject {
    fn event(state: &mut Self, _: &XdgPopup, event: xdg_popup::Event, tag: &PopupTag, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            xdg_popup::Event::Configure { x, y, width, height } => {
                info!("popup {} configure pos=({x},{y}) size={width}x{height}", tag.0);
                if let Some(p) = state.popups.iter_mut().find(|p| p.id == tag.0) {
                    if width > 0 && height > 0 {
                        p.w = width;
                        p.h = height;
                    }
                }
            }
            xdg_popup::Event::PopupDone => {
                info!("popup {} dismissed by compositor", tag.0);
                if let Some(i) = state.popups.iter().position(|p| p.id == tag.0) {
                    let p = state.popups.remove(i);
                    p.popup.destroy();
                    p.xdg_surface.destroy();
                    p.surface.destroy();
                }
            }
            xdg_popup::Event::Repositioned { token } => {
                info!("popup {} repositioned token={token}", tag.0);
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgPositioner, ()> for Subject {
    fn event(_: &mut Self, _: &XdgPositioner, _: xdg_positioner::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZxdgDecorationManagerV1, ()> for Subject {
    fn event(_: &mut Self, _: &ZxdgDecorationManagerV1, _: <ZxdgDecorationManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZxdgToplevelDecorationV1, ()> for Subject {
    fn event(state: &mut Self, _: &ZxdgToplevelDecorationV1, event: zxdg_toplevel_decoration_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event {
            state.deco_effective = Some(match mode {
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ServerSide) => "server",
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ClientSide) => "client",
                _ => "?",
            });
            info!("decoration configure -> {:?}", state.deco_effective);
            if state.configured && state.mapped {
                state.draw_main();
            }
        }
    }
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
            info!("preferred fractional scale = {scale} ({:.3})", scale as f32 / 120.0);
            if state.configured && state.mapped {
                state.draw_main();
            }
        }
    }
}

impl Dispatch<WpTearingControlManagerV1, ()> for Subject {
    fn event(_: &mut Self, _: &WpTearingControlManagerV1, _: <WpTearingControlManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WpTearingControlV1, ()> for Subject {
    fn event(_: &mut Self, _: &WpTearingControlV1, _: <WpTearingControlV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

// The manager announces the sizes the compositor would like at bind time; the
// `done` that follows just ends that burst.
impl Dispatch<XdgToplevelIconManagerV1, ()> for Subject {
    fn event(
        state: &mut Self,
        _: &XdgToplevelIconManagerV1,
        event: xdg_toplevel_icon_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel_icon_manager_v1::Event::IconSize { size } = event {
            state.icon_sizes.push(size);
        }
    }
}
impl Dispatch<XdgToplevelIconV1, ()> for Subject {
    fn event(_: &mut Self, _: &XdgToplevelIconV1, _: <XdgToplevelIconV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpSinglePixelBufferManagerV1, ()> for Subject {
    fn event(_: &mut Self, _: &WpSinglePixelBufferManagerV1, _: <WpSinglePixelBufferManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<WlBuffer, SpBuf> for Subject {
    fn event(_: &mut Self, buffer: &WlBuffer, event: wl_buffer::Event, _: &SpBuf, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

/// Colour per icon size, so the size the compositor chose is readable off the
/// pixels it drew: 0=red, 1=amber, 2=green, 3=blue, 4=magenta, then repeat.
fn icon_tint(index: usize) -> u32 {
    const TINTS: [u32; 5] = [0xFFE0_4038, 0xFFE0_A020, 0xFF30_C060, 0xFF3070_E0, 0xFFC0_40C0];
    TINTS[index % TINTS.len()]
}

/// Scale an opaque `0xFFRRGGBB` to alpha `a` with the channels PREMULTIPLIED,
/// which is what `wl_shm`'s `argb8888` means.
fn premultiply(argb: u32, a: u8) -> u32 {
    let scale = |c: u32| ((c & 0xFF) * a as u32 / 255) & 0xFF;
    ((a as u32) << 24) | (scale(argb >> 16) << 16) | (scale(argb >> 8) << 8) | scale(argb)
}
