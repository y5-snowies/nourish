//! y5 pen / stylus stress / visualiser.
//!
//! A real `zwp_tablet_v2` client that exercises the compositor's tablet path and
//! shows every stylus parameter it receives, so you can verify native pen support
//! works AND — with `--no-tablet` — that a client which does NOT bind the tablet
//! protocol falls back to the compositor's pointer emulation instead. It is the pen
//! sibling of `developer.tool.touch/touch.stress`.
//!
//! It binds the WHOLE `zwp_tablet_tool_v2` surface — proximity in/out, tip down/up,
//! motion, pressure, distance, tilt (x/y), rotation, slider, wheel, per-tool buttons,
//! tool *type* (pen / eraser / brush / pencil / airbrush / …), hardware serial/id and
//! capabilities — plus the tablet pad (buttons / ring / strip). Every stroke is inked
//! onto the surface with a nib whose radius tracks pressure, and eraser-type tools
//! erase, so tilt / pressure / eraser-mode are all visible at a glance.
//!
//! Scenarios:
//!   (default)      binds tablet + wl_pointer + wl_keyboard  → full pen support
//!   --no-tablet    binds pointer + keyboard only            → pointer-emulation path
//!   --no-pointer   binds tablet only                        → pure zwp_tablet_v2
//!
//! Keys (when a keyboard is bound): `t` toggle tablet live · `c` clear ink+log · `q`/Esc quit.
//! Run against a nested y5 (or any compositor): `WAYLAND_DISPLAY=... ./pen-stress`.

mod canvas;
mod font;

use canvas::{color, Canvas};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    backend::ObjectId,
    event_created_child,
    globals::registry_queue_init,
    protocol::{
        wl_keyboard::WlKeyboard, wl_output::WlOutput, wl_pointer::WlPointer, wl_seat::WlSeat,
        wl_shm::Format, wl_surface::WlSurface,
    },
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2::{self, ZwpTabletManagerV2},
    zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2, EVT_RING_OPCODE, EVT_STRIP_OPCODE},
    zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
    zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2, EVT_GROUP_OPCODE},
    zwp_tablet_seat_v2::{
        self, ZwpTabletSeatV2, EVT_PAD_ADDED_OPCODE, EVT_TABLET_ADDED_OPCODE, EVT_TOOL_ADDED_OPCODE,
    },
    zwp_tablet_tool_v2::{self, Capability as ToolCap, Type as ToolType, ZwpTabletToolV2},
    zwp_tablet_v2::{self, ZwpTabletV2},
};

const INIT_W: u32 = 980;
const INIT_H: u32 = 700;
const TRAIL: usize = 24; // hover-trail points kept per tool
const LOG_MAX: usize = 14; // event-log lines shown
const INK_MAX: usize = 6000; // ink segments retained (ring buffer)

/// One recorded ink segment (a single motion step while the tip is down).
struct Ink {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    r: i32,
    argb: u32,
    erase: bool,
}

/// Live state of one `zwp_tablet_tool_v2` (a stylus / eraser / brush / …).
struct PenTool {
    tool_type: Option<ToolType>,
    caps: BTreeSet<&'static str>,
    hw_serial: Option<u64>,
    hw_wacom: Option<u64>,
    color: u32,

    x: f64,
    y: f64,
    pressure: f64, // 0..1
    distance: f64, // 0..1  (hover gap; 0 = touching)
    tilt_x: f64,   // degrees, +/- from vertical
    tilt_y: f64,
    rotation: f64,   // degrees (barrel rotation)
    slider: f64,     // -1..1  (finger slider, e.g. airbrush)
    wheel_deg: f64,  // accumulated wheel degrees
    wheel_clicks: i32,
    proximity: bool, // within sensor range
    down: bool,      // tip in contact
    buttons: BTreeSet<u32>,

    trail: VecDeque<(f64, f64)>,
    ink_prev: Option<(f64, f64)>, // last inked point while down
}

impl PenTool {
    fn new() -> Self {
        PenTool {
            tool_type: None,
            caps: BTreeSet::new(),
            hw_serial: None,
            hw_wacom: None,
            color: color::CYAN,
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            distance: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            slider: 0.0,
            wheel_deg: 0.0,
            wheel_clicks: 0,
            proximity: false,
            down: false,
            buttons: BTreeSet::new(),
            trail: VecDeque::new(),
            ink_prev: None,
        }
    }

    fn is_eraser(&self) -> bool {
        matches!(self.tool_type, Some(ToolType::Eraser))
    }

    fn type_name(&self) -> &'static str {
        match self.tool_type {
            Some(ToolType::Pen) => "PEN",
            Some(ToolType::Eraser) => "ERASER",
            Some(ToolType::Brush) => "BRUSH",
            Some(ToolType::Pencil) => "PENCIL",
            Some(ToolType::Airbrush) => "AIRBRUSH",
            Some(ToolType::Finger) => "FINGER",
            Some(ToolType::Mouse) => "MOUSE",
            Some(ToolType::Lens) => "LENS",
            _ => "TOOL",
        }
    }

    /// Stable colour by tool type (eraser is unmistakable).
    fn color_for(&self) -> u32 {
        match self.tool_type {
            Some(ToolType::Eraser) => color::MAGENTA,
            Some(ToolType::Brush) => color::GREEN,
            Some(ToolType::Pencil) => color::YELLOW,
            Some(ToolType::Airbrush) => color::ORANGE,
            Some(ToolType::Mouse) | Some(ToolType::Lens) => color::LTGREY,
            _ => color::CYAN,
        }
    }
}

/// Aggregate of any tablet pad(s): the physical buttons / ring / strip on the tablet
/// body (not the stylus). Kept flat — a stress viewer only needs the latest values.
#[derive(Default)]
struct PadState {
    present: bool,
    focused: bool,            // pad.enter / pad.leave
    path: Option<String>,     // pad.path (device sysfs path)
    button_count: u32,        // pad.buttons (announced total)
    group_buttons: usize,     // group.buttons (announced array size)
    modes: u32,               // group.modes
    mode: u32,                // group.mode_switch (current)
    buttons: BTreeSet<u32>,   // live pressed
    ring_angle: Option<f64>,  // degrees, None when the finger left the ring
    ring_finger: bool,        // ring source == finger
    strip_pos: Option<f64>,   // 0..1, None when the finger left the strip
    strip_finger: bool,       // strip source == finger
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,

    width: u32,
    height: u32,
    configured: bool,
    exit: bool,

    seat: Option<WlSeat>,
    tablet_manager: Option<ZwpTabletManagerV2>,
    tablet_seat: Option<ZwpTabletSeatV2>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    /// Whether the tablet seat should be live. `--no-tablet` starts false; `t` toggles.
    want_tablet: bool,
    want_pointer: bool,

    tools: HashMap<ObjectId, PenTool>,
    pad: PadState,
    ink: VecDeque<Ink>,
    pointer_pos: Option<(f64, f64)>,
    pointer_down: bool,
    log: VecDeque<String>,
}

impl App {
    fn note(&mut self, line: String) {
        eprintln!("[pen] {line}");
        self.log.push_back(line);
        while self.log.len() > LOG_MAX {
            self.log.pop_front();
        }
    }

    /// Bring the tablet-seat binding in line with `want_tablet`. Destroying the seat
    /// drops all our tool objects, so the compositor's `has tablet client` check flips
    /// and (if it emulates) the pointer path takes over — the pen mirror of the touch
    /// tool's live `wl_touch` toggle.
    fn reconcile_tablet(&mut self, qh: &QueueHandle<Self>) {
        match (self.want_tablet, self.tablet_seat.is_some()) {
            (true, false) => {
                let (Some(mgr), Some(seat)) = (self.tablet_manager.clone(), self.seat.clone())
                else {
                    return; // no manager global, or no wl_seat yet — retried on new_seat
                };
                self.tablet_seat = Some(mgr.get_tablet_seat(&seat, qh, ()));
                self.note("tablet seat BOUND (native zwp_tablet_v2)".into());
            }
            (false, true) => {
                if let Some(ts) = self.tablet_seat.take() {
                    ts.destroy();
                }
                self.tools.clear();
                self.pad = PadState::default();
                self.note("tablet seat RELEASED (pointer-emulation path)".into());
            }
            _ => {}
        }
        if self.want_pointer && self.pointer.is_none() {
            if let Some(seat) = self.seat.clone() {
                self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
            }
        }
    }

    /// End-of-frame: commit the pending stroke for this tool. Tablet events arrive as
    /// bursts terminated by `frame`, so we ink once per frame rather than per axis.
    fn tool_frame(&mut self, id: &ObjectId) {
        let (seg, erase, color, cur, down, proximity) = {
            let Some(t) = self.tools.get_mut(id) else { return };
            let cur = (t.x, t.y);
            let mut seg = None;
            if t.down {
                // Nib radius grows with pressure; eraser has a fat fixed nib.
                let r = if t.is_eraser() {
                    9
                } else {
                    1 + (t.pressure * 7.0).round() as i32
                };
                let from = t.ink_prev.unwrap_or(cur);
                seg = Some((from, cur, r));
                t.ink_prev = Some(cur);
            } else {
                t.ink_prev = None;
            }
            if t.proximity {
                t.trail.push_back(cur);
                while t.trail.len() > TRAIL {
                    t.trail.pop_front();
                }
            }
            (seg, t.is_eraser(), t.color, cur, t.down, t.proximity)
        };
        let _ = (cur, down, proximity);
        if let Some((from, to, r)) = seg {
            self.ink.push_back(Ink {
                x0: from.0,
                y0: from.1,
                x1: to.0,
                y1: to.1,
                r,
                argb: color,
                erase,
            });
            while self.ink.len() > INK_MAX {
                self.ink.pop_front();
            }
        }
    }

    fn tick(&mut self) {
        // Age hover trails so a lifted-then-idle trail fades — purely cosmetic.
        for t in self.tools.values_mut() {
            if !t.down && t.trail.len() > 1 {
                t.trail.pop_front();
            }
        }
        if self.configured {
            self.draw();
        }
    }

    fn draw(&mut self) {
        let (w, h) = (self.width.max(1) as i32, self.height.max(1) as i32);
        let stride = w * 4;
        let bg = 0xFF10_1418u32;
        let (buffer, slice) = match self.pool.create_buffer(w, h, stride, Format::Argb8888) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[pen] buffer alloc failed: {e}");
                return;
            }
        };
        {
            let mut cv = Canvas::new(slice, w, h);
            cv.clear(bg);

            // ── ink layer (drawn first, everything else sits on top) ──────────────
            for seg in &self.ink {
                let argb = if seg.erase { bg } else { seg.argb };
                cv.thick_line(seg.x0 as i32, seg.y0 as i32, seg.x1 as i32, seg.y1 as i32, seg.r, argb);
            }

            cv.frame(0, 0, w, h, 2, color::DKGREY);

            // ── header + mode banner ──────────────────────────────────────────────
            let mode = match (self.tablet_seat.is_some(), self.pointer.is_some()) {
                (true, _) => "TABLET BOUND  (native zwp_tablet_v2)",
                (false, true) => "POINTER-ONLY  (compositor pointer-emulation)",
                (false, false) => "NO INPUT BOUND",
            };
            cv.rect(0, 0, w, 60, color::PANEL);
            font::text(&mut cv, 8, 8, 2, color::WHITE, "y5 PEN STRESS");
            font::text(&mut cv, 8, 30, 1, color::LTGREY, mode);
            font::text(
                &mut cv,
                8,
                44,
                1,
                color::GREY,
                &format!("TOOLS {}   INK {}   T=TOGGLE  C=CLEAR  Q=QUIT", self.tools.len(), self.ink.len()),
            );

            // ── per-tool live stats (top-right), + on-canvas overlays ─────────────
            let mut sy = 66;
            for t in self.tools.values() {
                // On-canvas: hover trail, nib ring (sized by pressure / distance),
                // crosshair, tilt vector, rotation tick, label.
                let mut prev: Option<(f64, f64)> = None;
                for &(tx, ty) in &t.trail {
                    if let Some((qx, qy)) = prev {
                        cv.line(qx as i32, qy as i32, tx as i32, ty as i32, 0xFF40_4850);
                    }
                    prev = Some((tx, ty));
                }
                if t.proximity {
                    let (x, y) = (t.x as i32, t.y as i32);
                    let nib = if t.down {
                        (4.0 + t.pressure * 22.0) as i32
                    } else {
                        // Hover: ring shrinks as the tip nears the surface.
                        (6.0 + t.distance * 26.0) as i32
                    };
                    cv.ring(x, y, nib, t.color);
                    if t.down {
                        cv.ring(x, y, (nib - 1).max(1), t.color);
                    }
                    cv.crosshair(x, y, 12, t.color);

                    // Tilt vector: lean direction + magnitude (deg → px).
                    if t.tilt_x != 0.0 || t.tilt_y != 0.0 {
                        let ex = x + (t.tilt_x / 90.0 * 46.0) as i32;
                        let ey = y + (t.tilt_y / 90.0 * 46.0) as i32;
                        cv.line(x, y, ex, ey, color::WHITE);
                        cv.disc(ex, ey, 2, color::WHITE);
                    }
                    // Barrel rotation tick.
                    if t.rotation != 0.0 {
                        let a = t.rotation.to_radians();
                        let rx = x + (a.sin() * 30.0) as i32;
                        let ry = y - (a.cos() * 30.0) as i32;
                        cv.line(x, y, rx, ry, color::LTGREY);
                    }
                    let tag = if t.down { format!("{} DOWN", t.type_name()) } else { t.type_name().to_string() };
                    font::text(&mut cv, x + nib + 4, y - 4, 1, t.color, &tag);
                }

                // Top-right stat block.
                let l1 = format!(
                    "{}{}  P {:>3}%  D {:>3}%",
                    t.type_name(),
                    if t.is_eraser() { " *ERASE*" } else { "" },
                    (t.pressure * 100.0) as i32,
                    (t.distance * 100.0) as i32,
                );
                let l2 = format!(
                    "TILT {:+.0},{:+.0}  ROT {:.0}  SLD {:+.2}",
                    t.tilt_x, t.tilt_y, t.rotation, t.slider,
                );
                let btns: Vec<String> = t.buttons.iter().map(|b| format!("{b}")).collect();
                let l3 = format!(
                    "WHEEL {:.0}/{}  BTN [{}]",
                    t.wheel_deg,
                    t.wheel_clicks,
                    btns.join(","),
                );
                let l4 = {
                    let caps: Vec<&str> = t.caps.iter().copied().collect();
                    let hw = t
                        .hw_serial
                        .map(|s| format!("SER {s:X}"))
                        .or_else(|| t.hw_wacom.map(|s| format!("WACOM {s:X}")))
                        .unwrap_or_default();
                    format!("CAP {}  {}", caps.join("/"), hw)
                };
                for l in [&l1, &l2, &l3, &l4] {
                    let tw = font::text_width(l, 1);
                    font::text(&mut cv, (w - tw - 10).max(8), sy, 1, t.color, l);
                    sy += 12;
                }
                sy += 6;
            }

            // ── pad panel (tablet body: buttons / ring / strip) ───────────────────
            if self.pad.present {
                let pad_btns: Vec<String> = self.pad.buttons.iter().map(|b| format!("{b}")).collect();
                let ring = match (self.pad.ring_angle, self.pad.ring_finger) {
                    (Some(a), f) => format!("{a:.0}{}", if f { "F" } else { "" }),
                    (None, _) => "-".into(),
                };
                let strip = match (self.pad.strip_pos, self.pad.strip_finger) {
                    (Some(p), f) => format!("{:.0}%{}", p * 100.0, if f { "F" } else { "" }),
                    (None, _) => "-".into(),
                };
                let l1 = format!(
                    "PAD{}  BTN {}/{}  MODE {}/{}",
                    if self.pad.focused { " *FOCUS*" } else { "" },
                    self.pad.button_count,
                    self.pad.group_buttons,
                    self.pad.mode,
                    self.pad.modes,
                );
                let l2 = format!("  BTN [{}]  RING {}  STRIP {}", pad_btns.join(","), ring, strip);
                font::text(&mut cv, 8, sy, 1, color::LTGREY, &l1);
                font::text(&mut cv, 8, sy + 12, 1, color::LTGREY, &l2);
            }

            // ── pointer marker (present in the pointer-emulation mode) ─────────────
            if let Some((px, py)) = self.pointer_pos {
                let col = if self.pointer_down { color::YELLOW } else { color::LTGREY };
                cv.crosshair(px as i32, py as i32, 14, col);
                cv.ring(px as i32, py as i32, if self.pointer_down { 16 } else { 10 }, col);
            }

            // ── event log (bottom) ────────────────────────────────────────────────
            let log_y = h - (LOG_MAX as i32 + 1) * 12 - 8;
            cv.rect(0, log_y - 14, w, (LOG_MAX as i32 + 2) * 12, color::PANEL);
            font::text(&mut cv, 8, log_y - 12, 1, color::GREY, "EVENT LOG");
            for (i, l) in self.log.iter().enumerate() {
                font::text(&mut cv, 8, log_y + i as i32 * 12, 1, color::LTGREY, l);
            }
        }
        self.window.wl_surface().damage_buffer(0, 0, w, h);
        if let Err(e) = buffer.attach_to(self.window.wl_surface()) {
            eprintln!("[pen] attach failed: {e}");
        }
        self.window.wl_surface().commit();
    }
}

// ── zwp_tablet_manager_v2 (no events) ───────────────────────────────────────────
impl Dispatch<ZwpTabletManagerV2, ()> for App {
    fn event(_: &mut Self, _: &ZwpTabletManagerV2, _: zwp_tablet_manager_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

// ── zwp_tablet_seat_v2: hands us tablets / tools / pads ─────────────────────────
impl Dispatch<ZwpTabletSeatV2, ()> for App {
    fn event(state: &mut Self, _: &ZwpTabletSeatV2, event: zwp_tablet_seat_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_tablet_seat_v2::Event::ToolAdded { id } => {
                state.tools.insert(id.id(), PenTool::new());
                state.note("TOOL added".into());
            }
            zwp_tablet_seat_v2::Event::TabletAdded { .. } => state.note("TABLET added".into()),
            zwp_tablet_seat_v2::Event::PadAdded { .. } => {
                state.pad.present = true;
                state.note("PAD added".into());
            }
            _ => {}
        }
    }
    event_created_child!(App, ZwpTabletSeatV2, [
        EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

// ── zwp_tablet_v2: device identity ──────────────────────────────────────────────
impl Dispatch<ZwpTabletV2, ()> for App {
    fn event(state: &mut Self, _: &ZwpTabletV2, event: zwp_tablet_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_tablet_v2::Event::Name { name } = event {
            state.note(format!("TABLET name {name}"));
        }
    }
}

// ── zwp_tablet_tool_v2: THE stylus — every parameter under test ──────────────────
impl Dispatch<ZwpTabletToolV2, ()> for App {
    fn event(state: &mut Self, tool: &ZwpTabletToolV2, event: zwp_tablet_tool_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwp_tablet_tool_v2::Event as E;
        let id = tool.id();
        match event {
            E::Type { tool_type } => {
                if let Ok(tt) = tool_type.into_result() {
                    if let Some(t) = state.tools.get_mut(&id) {
                        t.tool_type = Some(tt);
                        t.color = t.color_for();
                    }
                    let name = state.tools.get(&id).map(|t| t.type_name()).unwrap_or("TOOL");
                    state.note(format!("TYPE {name}"));
                }
            }
            E::HardwareSerial { hardware_serial_hi, hardware_serial_lo } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.hw_serial = Some(((hardware_serial_hi as u64) << 32) | hardware_serial_lo as u64);
                }
            }
            E::HardwareIdWacom { hardware_id_hi, hardware_id_lo } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.hw_wacom = Some(((hardware_id_hi as u64) << 32) | hardware_id_lo as u64);
                }
            }
            E::Capability { capability } => {
                if let Ok(cap) = capability.into_result() {
                    let name = match cap {
                        ToolCap::Tilt => "TILT",
                        ToolCap::Pressure => "PRESSURE",
                        ToolCap::Distance => "DISTANCE",
                        ToolCap::Rotation => "ROTATION",
                        ToolCap::Slider => "SLIDER",
                        ToolCap::Wheel => "WHEEL",
                        _ => "?",
                    };
                    if let Some(t) = state.tools.get_mut(&id) {
                        t.caps.insert(name);
                    }
                }
            }
            E::Done => state.note("TOOL done (caps settled)".into()),
            E::ProximityIn { .. } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.proximity = true;
                }
                state.note("PROX IN".into());
            }
            E::ProximityOut => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.proximity = false;
                    t.down = false;
                    t.ink_prev = None;
                    t.trail.clear();
                    t.pressure = 0.0;
                    t.distance = 1.0;
                }
                state.note("PROX OUT".into());
            }
            E::Down { .. } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.down = true;
                    t.ink_prev = None;
                }
                state.note("DOWN (tip contact)".into());
            }
            E::Up => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.down = false;
                    t.ink_prev = None;
                }
                state.note("UP".into());
            }
            E::Motion { x, y } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.x = x;
                    t.y = y;
                }
            }
            E::Pressure { pressure } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.pressure = pressure as f64 / 65535.0;
                }
            }
            E::Distance { distance } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.distance = distance as f64 / 65535.0;
                }
            }
            E::Tilt { tilt_x, tilt_y } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.tilt_x = tilt_x;
                    t.tilt_y = tilt_y;
                }
            }
            E::Rotation { degrees } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.rotation = degrees;
                }
            }
            E::Slider { position } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.slider = position as f64 / 65535.0;
                }
            }
            E::Wheel { degrees, clicks } => {
                if let Some(t) = state.tools.get_mut(&id) {
                    t.wheel_deg += degrees;
                    t.wheel_clicks += clicks;
                }
                state.note(format!("WHEEL {degrees:.0} deg / {clicks} clicks"));
            }
            E::Button { button, state: bstate, .. } => {
                let pressed = matches!(bstate.into_result(), Ok(zwp_tablet_tool_v2::ButtonState::Pressed));
                if let Some(t) = state.tools.get_mut(&id) {
                    if pressed {
                        t.buttons.insert(button);
                    } else {
                        t.buttons.remove(&button);
                    }
                }
                state.note(format!("BTN {button} {}", if pressed { "press" } else { "release" }));
            }
            E::Frame { .. } => state.tool_frame(&id),
            E::Removed => {
                state.tools.remove(&id);
                tool.destroy();
                state.note("TOOL removed".into());
            }
            _ => {}
        }
    }
}

// ── zwp_tablet_pad_v2 + group / ring / strip: the tablet body controls ───────────
impl Dispatch<ZwpTabletPadV2, ()> for App {
    fn event(state: &mut Self, _: &ZwpTabletPadV2, event: zwp_tablet_pad_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwp_tablet_pad_v2::Event as E;
        match event {
            E::Path { path } => {
                state.note(format!("PAD path {path}"));
                state.pad.path = Some(path);
            }
            E::Buttons { buttons } => {
                state.pad.button_count = buttons;
                state.note(format!("PAD buttons {buttons}"));
            }
            E::Button { button, state: bstate, .. } => {
                let pressed = matches!(bstate.into_result(), Ok(zwp_tablet_pad_v2::ButtonState::Pressed));
                if pressed {
                    state.pad.buttons.insert(button);
                } else {
                    state.pad.buttons.remove(&button);
                }
                state.note(format!("PAD BTN {button} {}", if pressed { "press" } else { "release" }));
            }
            E::Enter { .. } => {
                state.pad.focused = true;
                state.note("PAD enter".into());
            }
            E::Leave { .. } => {
                state.pad.focused = false;
                state.note("PAD leave".into());
            }
            E::Done => state.note("PAD done".into()),
            E::Removed => {
                state.pad = PadState::default();
                state.note("PAD removed".into());
            }
            _ => {}
        }
    }
    event_created_child!(App, ZwpTabletPadV2, [
        EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for App {
    fn event(state: &mut Self, _: &ZwpTabletPadGroupV2, event: zwp_tablet_pad_group_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwp_tablet_pad_group_v2::Event as E;
        match event {
            E::Modes { modes } => {
                state.pad.modes = modes;
                state.note(format!("PAD group modes {modes}"));
            }
            E::ModeSwitch { mode, .. } => {
                state.pad.mode = mode;
                state.note(format!("PAD mode -> {mode}"));
            }
            // group.buttons is a wl_array of native-endian u32 indices.
            E::Buttons { buttons } => state.pad.group_buttons = buttons.len() / 4,
            _ => {}
        }
    }
    event_created_child!(App, ZwpTabletPadGroupV2, [
        EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadRingV2, ()> for App {
    fn event(state: &mut Self, _: &ZwpTabletPadRingV2, event: zwp_tablet_pad_ring_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_tablet_pad_ring_v2::Event::Source { source } => {
                state.pad.ring_finger = matches!(source.into_result(), Ok(zwp_tablet_pad_ring_v2::Source::Finger));
            }
            zwp_tablet_pad_ring_v2::Event::Angle { degrees } => state.pad.ring_angle = Some(degrees),
            zwp_tablet_pad_ring_v2::Event::Stop => state.pad.ring_angle = None,
            _ => {}
        }
    }
}

impl Dispatch<ZwpTabletPadStripV2, ()> for App {
    fn event(state: &mut Self, _: &ZwpTabletPadStripV2, event: zwp_tablet_pad_strip_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwp_tablet_pad_strip_v2::Event::Source { source } => {
                state.pad.strip_finger = matches!(source.into_result(), Ok(zwp_tablet_pad_strip_v2::Source::Finger));
            }
            zwp_tablet_pad_strip_v2::Event::Position { position } => state.pad.strip_pos = Some(position as f64 / 65535.0),
            zwp_tablet_pad_strip_v2::Event::Stop => state.pad.strip_pos = None,
            _ => {}
        }
    }
}

// ── wl_pointer: shows the compositor's emulation when the tablet isn't bound ──────
impl PointerHandler for App {
    fn pointer_frame(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _p: &WlPointer, events: &[PointerEvent]) {
        for e in events {
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_pos = Some((e.position.0, e.position.1));
                }
                PointerEventKind::Leave { .. } => self.pointer_pos = None,
                PointerEventKind::Press { .. } => {
                    self.pointer_down = true;
                    self.note(format!("PTR press ({:.0},{:.0})", e.position.0, e.position.1));
                }
                PointerEventKind::Release { .. } => {
                    self.pointer_down = false;
                    self.note("PTR release".into());
                }
                _ => {}
            }
        }
    }
}
delegate_pointer!(App);

// ── wl_keyboard: live toggles ────────────────────────────────────────────────────
impl KeyboardHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: &WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: &WlSurface, _: u32) {}
    fn press_key(&mut self, _c: &Connection, qh: &QueueHandle<Self>, _: &WlKeyboard, _: u32, ev: KeyEvent) {
        match ev.keysym {
            Keysym::t | Keysym::T => {
                self.want_tablet = !self.want_tablet;
                self.reconcile_tablet(qh);
            }
            Keysym::c | Keysym::C => {
                self.ink.clear();
                self.log.clear();
                for t in self.tools.values_mut() {
                    t.trail.clear();
                    t.ink_prev = None;
                }
            }
            Keysym::q | Keysym::Q | Keysym::Escape => self.exit = true,
            _ => {}
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: Modifiers, _: u32) {}
}
delegate_keyboard!(App);

// ── seat: bind pointer / keyboard, and (re)establish the tablet seat ─────────────
impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat) {
        self.seat = Some(seat);
        self.reconcile_tablet(qh);
    }
    fn new_capability(&mut self, _c: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
        self.seat = Some(seat.clone());
        if cap == Capability::Pointer && self.want_pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        self.reconcile_tablet(qh);
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, cap: Capability) {
        match cap {
            Capability::Pointer => {
                if let Some(p) = self.pointer.take() {
                    p.release();
                }
            }
            Capability::Keyboard => {
                if let Some(k) = self.keyboard.take() {
                    k.release();
                }
            }
            _ => {}
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}
delegate_seat!(App);

// ── xdg window ──────────────────────────────────────────────────────────────────
impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _serial: u32) {
        if let (Some(w), Some(h)) = configure.new_size {
            self.width = w.get();
            self.height = h.get();
        }
        self.configured = true;
        self.draw();
    }
}
delegate_xdg_window!(App);
delegate_xdg_shell!(App);

// ── compositor / output / shm / registry boilerplate ────────────────────────────
impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wayland_client::protocol::wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
}
delegate_compositor!(App);

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}
delegate_output!(App);

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
delegate_shm!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
delegate_registry!(App);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "pen-stress [--no-tablet] [--no-pointer]\n  --no-tablet   don't bind zwp_tablet_v2 (exercise compositor pointer-emulation)\n  --no-pointer  don't bind wl_pointer (pure tablet)\nKeys: t=toggle tablet, c=clear ink+log, q/Esc=quit"
        );
        return;
    }
    let want_tablet = !args.iter().any(|a| a == "--no-tablet");
    let want_pointer = !args.iter().any(|a| a == "--no-pointer");

    let conn = Connection::connect_to_env().expect("connect to wayland");
    let (globals, event_queue) = registry_queue_init::<App>(&conn).expect("registry init");
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .expect("insert wayland source");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg_shell");
    let pool = SlotPool::new((INIT_W * INIT_H * 4) as usize, &shm).expect("slot pool");

    // Bind the tablet manager global up front (if the compositor advertises it). The
    // per-seat tablet seat is created lazily once a wl_seat appears (see reconcile).
    let tablet_manager = if want_tablet {
        match globals.bind::<ZwpTabletManagerV2, _, _>(&qh, 1..=1, ()) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("[pen] no zwp_tablet_manager_v2 — compositor lacks tablet support ({e}); pointer-only");
                None
            }
        }
    } else {
        None
    };

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("y5 pen stress");
    window.set_app_id("y5.pen.stress");
    window.set_min_size(Some((480, 360)));
    window.commit();

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        width: INIT_W,
        height: INIT_H,
        configured: false,
        exit: false,
        seat: None,
        tablet_manager,
        tablet_seat: None,
        pointer: None,
        keyboard: None,
        want_tablet,
        want_pointer,
        tools: HashMap::new(),
        pad: PadState::default(),
        ink: VecDeque::new(),
        pointer_pos: None,
        pointer_down: false,
        log: VecDeque::new(),
    };

    // ~40 Hz timer to animate trails and keep the surface fresh.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(25)), |_, _, app| {
            app.tick();
            TimeoutAction::ToDuration(Duration::from_millis(25))
        })
        .expect("insert timer");

    while !app.exit {
        event_loop.dispatch(Duration::from_millis(50), &mut app).unwrap();
    }
}
