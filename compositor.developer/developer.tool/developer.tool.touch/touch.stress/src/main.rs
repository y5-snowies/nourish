//! y5 touch stress / visualiser.
//!
//! A real `wl_touch` client that exercises the compositor's touch path and shows
//! what it receives, so you can verify native multi-touch works AND — with
//! `--no-touch` — that a client which does NOT bind `wl_touch` gets the
//! compositor's pointer emulation instead. It is also a scaffold for building
//! future touch features (text selection, …): the content area renders selectable
//! text lines, and every event is logged on-screen.
//!
//! Scenarios:
//!   (default)      binds wl_touch + wl_pointer + wl_keyboard  → full touch support
//!   --no-touch     binds pointer + keyboard only              → pointer-emulation path
//!   --no-pointer   binds touch only                           → pure wl_touch
//!
//! Keys (when a keyboard is bound): `t` toggle wl_touch live · `c` clear · `q`/Esc quit.
//! Run against a nested y5 (or any compositor): `WAYLAND_DISPLAY=... ./touch-stress`.

mod canvas;
mod font;

use canvas::{color, Canvas};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_touch, delegate_xdg_shell, delegate_xdg_window,
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
        touch::TouchHandler,
        Capability, SeatHandler, SeatState,
    },
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{
        slot::SlotPool,
        Shm, ShmHandler,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{
        wl_keyboard::WlKeyboard, wl_output::WlOutput, wl_pointer::WlPointer, wl_seat::WlSeat,
        wl_shm::Format, wl_surface::WlSurface, wl_touch::WlTouch,
    },
    Connection, QueueHandle,
};

const INIT_W: u32 = 900;
const INIT_H: u32 = 640;
const TRAIL: usize = 24; // trail points kept per finger
const LOG_MAX: usize = 16; // event-log lines shown

/// One active finger.
struct Finger {
    x: f64,
    y: f64,
    trail: VecDeque<(f64, f64)>,
    major: f64,
    minor: f64,
    orient: f64,
    color: u32,
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
    touch: Option<WlTouch>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    /// Whether to (re)bind wl_touch. `--no-touch` starts false; `t` toggles it.
    want_touch: bool,
    want_pointer: bool,

    fingers: HashMap<i32, Finger>,
    pointer_pos: Option<(f64, f64)>,
    pointer_down: bool,
    log: VecDeque<String>,
}

impl App {
    fn note(&mut self, line: String) {
        eprintln!("[touch] {line}");
        self.log.push_back(line);
        while self.log.len() > LOG_MAX {
            self.log.pop_front();
        }
    }

    /// Per-finger colour (stable by slot).
    fn slot_color(id: i32) -> u32 {
        const PAL: [u32; 6] = [
            color::CYAN, color::MAGENTA, color::YELLOW, color::GREEN, color::RED, color::BLUE,
        ];
        PAL[(id.rem_euclid(6)) as usize]
    }

    /// Bring the bound-input set in line with `want_touch`/`want_pointer` for the
    /// current seat — the live toggle re-issues `get_touch`/releases it, so the
    /// compositor's `client_has_touch` flips and pointer-emulation kicks in/out.
    fn reconcile_inputs(&mut self, qh: &QueueHandle<Self>) {
        let Some(seat) = self.seat.clone() else { return };
        match (self.want_touch, self.touch.is_some()) {
            (true, false) => {
                self.touch = self.seat_state.get_touch(qh, &seat).ok();
                self.note("wl_touch BOUND".into());
            }
            (false, true) => {
                if let Some(t) = self.touch.take() {
                    t.release();
                }
                self.fingers.clear();
                self.note("wl_touch RELEASED (pointer-emulation path)".into());
            }
            _ => {}
        }
        if self.want_pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    /// Age each finger's trail slightly (called on the timer) so a lifted-then-idle
    /// trail doesn't linger forever — purely cosmetic.
    fn tick(&mut self) {
        for f in self.fingers.values_mut() {
            if f.trail.len() > 1 {
                f.trail.pop_front();
            }
        }
        if self.configured {
            self.draw();
        }
    }

    fn draw(&mut self) {
        let (w, h) = (self.width.max(1) as i32, self.height.max(1) as i32);
        let stride = w * 4;
        let (buffer, slice) = match self.pool.create_buffer(w, h, stride, Format::Argb8888) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[touch] buffer alloc failed: {e}");
                return;
            }
        };
        {
            let mut cv = Canvas::new(slice, w, h);
            cv.clear(0xFF10_1418);
            cv.frame(0, 0, w, h, 2, color::DKGREY);

            // Header + mode banner.
            let mode = match (self.touch.is_some(), self.pointer.is_some()) {
                (true, _) => "TOUCH BOUND  (native wl_touch)",
                (false, true) => "POINTER-ONLY  (compositor pointer-emulation)",
                (false, false) => "NO INPUT BOUND",
            };
            font::text(&mut cv, 8, 8, 2, color::WHITE, "y5 TOUCH STRESS");
            font::text(&mut cv, 8, 30, 1, color::LTGREY, mode);
            font::text(
                &mut cv,
                8,
                44,
                1,
                color::GREY,
                &format!("FINGERS {}    T=TOGGLE  C=CLEAR  Q=QUIT", self.fingers.len()),
            );

            // Selectable-text content area (a surface for future text-selection work).
            let cx0 = 8;
            let cy0 = 66;
            cv.frame(cx0, cy0, w - 16, 120, 1, color::BTN);
            font::text(&mut cv, cx0 + 6, cy0 + 6, 1, color::GREY, "CONTENT (for future text-select features):");
            let lines = [
                "The quick brown fox jumps over the lazy dog.",
                "Touch and drag across this text to prototype selection.",
                "Slot ids, positions, shape + orientation all logged below.",
            ];
            for (i, l) in lines.iter().enumerate() {
                font::text(&mut cv, cx0 + 6, cy0 + 24 + i as i32 * 16, 1, color::WHITE, l);
            }

            // Pointer marker (present in the pointer-emulation mode).
            if let Some((px, py)) = self.pointer_pos {
                let col = if self.pointer_down { color::YELLOW } else { color::LTGREY };
                cv.crosshair(px as i32, py as i32, 14, col);
                cv.ring(px as i32, py as i32, if self.pointer_down { 16 } else { 10 }, col);
            }

            // Each finger: trail, ring sized by shape, crosshair, slot id.
            for (id, f) in &self.fingers {
                let mut prev: Option<(f64, f64)> = None;
                for &(tx, ty) in &f.trail {
                    if let Some((qx, qy)) = prev {
                        line(&mut cv, qx as i32, qy as i32, tx as i32, ty as i32, 0xFF40_4850);
                    }
                    prev = Some((tx, ty));
                }
                let (x, y) = (f.x as i32, f.y as i32);
                let r = ((f.major.max(f.minor)).max(24.0) / 2.0) as i32;
                cv.ring(x, y, r, f.color);
                cv.ring(x, y, r.max(1) - 1, f.color);
                cv.crosshair(x, y, 10, f.color);
                font::text(&mut cv, x + r + 4, y - 4, 1, f.color, &format!("#{id}"));
            }

            // Event log at the bottom.
            let log_y = h - (LOG_MAX as i32 + 1) * 12 - 8;
            font::text(&mut cv, 8, log_y - 12, 1, color::GREY, "EVENT LOG");
            for (i, l) in self.log.iter().enumerate() {
                font::text(&mut cv, 8, log_y + i as i32 * 12, 1, color::LTGREY, l);
            }
        }
        self.window.wl_surface().damage_buffer(0, 0, w, h);
        if let Err(e) = buffer.attach_to(self.window.wl_surface()) {
            eprintln!("[touch] attach failed: {e}");
        }
        self.window.wl_surface().commit();
    }
}

/// Bresenham line into the canvas (trail segments).
fn line(cv: &mut Canvas, x0: i32, y0: i32, x1: i32, y1: i32, argb: u32) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    loop {
        cv.put(x, y, argb);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

// ── wl_touch: the feature under test ────────────────────────────────────────────
impl TouchHandler for App {
    fn down(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _t: &WlTouch, _serial: u32, _time: u32, _surface: WlSurface, id: i32, position: (f64, f64)) {
        let color = Self::slot_color(id);
        let mut trail = VecDeque::new();
        trail.push_back(position);
        self.fingers.insert(id, Finger { x: position.0, y: position.1, trail, major: 24.0, minor: 24.0, orient: 0.0, color });
        self.note(format!("DOWN  #{id}  ({:.0},{:.0})", position.0, position.1));
    }
    fn up(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _t: &WlTouch, _serial: u32, _time: u32, id: i32) {
        self.fingers.remove(&id);
        self.note(format!("UP    #{id}"));
    }
    fn motion(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _t: &WlTouch, _time: u32, id: i32, position: (f64, f64)) {
        if let Some(f) = self.fingers.get_mut(&id) {
            f.x = position.0;
            f.y = position.1;
            f.trail.push_back(position);
            while f.trail.len() > TRAIL {
                f.trail.pop_front();
            }
        }
    }
    fn shape(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _t: &WlTouch, id: i32, major: f64, minor: f64) {
        if let Some(f) = self.fingers.get_mut(&id) {
            f.major = major;
            f.minor = minor;
        }
        self.note(format!("SHAPE #{id}  {major:.0}x{minor:.0}"));
    }
    fn orientation(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _t: &WlTouch, id: i32, orientation: f64) {
        if let Some(f) = self.fingers.get_mut(&id) {
            f.orient = orientation;
        }
        self.note(format!("ORIENT #{id}  {orientation:.0}"));
    }
    fn cancel(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _t: &WlTouch) {
        self.fingers.clear();
        self.note("CANCEL (compositor claimed the sequence)".into());
    }
}
delegate_touch!(App);

// ── wl_pointer: shows the compositor's emulation when touch isn't bound ──────────
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
                    self.note(format!("PTR   press  ({:.0},{:.0})", e.position.0, e.position.1));
                }
                PointerEventKind::Release { .. } => {
                    self.pointer_down = false;
                    self.note("PTR   release".into());
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
                self.want_touch = !self.want_touch;
                self.reconcile_inputs(qh);
            }
            Keysym::c | Keysym::C => {
                self.fingers.clear();
                self.log.clear();
            }
            Keysym::q | Keysym::Q | Keysym::Escape => self.exit = true,
            _ => {}
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: Modifiers, _: u32) {}
}
delegate_keyboard!(App);

// ── seat: bind touch/pointer/keyboard per the chosen mode ───────────────────────
impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(&mut self, _c: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
        self.seat = Some(seat.clone());
        if cap == Capability::Touch && self.want_touch && self.touch.is_none() {
            self.touch = self.seat_state.get_touch(qh, &seat).ok();
        }
        if cap == Capability::Pointer && self.want_pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, cap: Capability) {
        match cap {
            Capability::Touch => {
                if let Some(t) = self.touch.take() {
                    t.release();
                }
            }
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
        eprintln!("touch-stress [--no-touch] [--no-pointer]\n  --no-touch   don't bind wl_touch (exercise compositor pointer-emulation)\n  --no-pointer don't bind wl_pointer (pure wl_touch)\nKeys: t=toggle touch, c=clear, q/Esc=quit");
        return;
    }
    let want_touch = !args.iter().any(|a| a == "--no-touch");
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

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("y5 touch stress");
    window.set_app_id("y5.touch.stress");
    window.set_min_size(Some((400, 300)));
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
        touch: None,
        pointer: None,
        keyboard: None,
        want_touch,
        want_pointer,
        fingers: HashMap::new(),
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
