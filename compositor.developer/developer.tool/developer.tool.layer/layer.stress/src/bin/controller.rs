//! The controller: a well-behaved xdg GUI window that spawns the subject as a child process
//! and drives it by writing [`Command`] lines to its stdin. Clickable buttons grouped by
//! layer-shell dimension, plus +/- steppers and a command-log panel.
//!
//! The subject binary is found next to this executable (`layer-stress-subject`).
//!
//! Run with `--selftest` to enumerate the button/command surface without connecting to
//! Wayland or spawning the subject (headless verification).

use std::io::Write as _;
use std::process::{Child, ChildStdin, Command as PCommand, Stdio};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_pointer, delegate_registry, delegate_seat,
    delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer::WlPointer, wl_seat::WlSeat, wl_shm::Format, wl_surface::WlSurface},
};

use layer_stress::canvas::{Canvas, color};
use layer_stress::diag;
use layer_stress::protocol::{Anchor, Command, ForeignCtl, Kbd, Layer, Region, ScaleReq};
use layer_stress::{font, info, warn};

const WIN_W: i32 = 800;
const WIN_H: i32 = 880;
const ANCHORS: [Anchor; 9] = [
    Anchor::Center,
    Anchor::Top,
    Anchor::Bottom,
    Anchor::Left,
    Anchor::Right,
    Anchor::TopLeft,
    Anchor::TopRight,
    Anchor::BottomLeft,
    Anchor::BottomRight,
];

/// A click action: either a command to forward to the subject, or a controller-side action.
#[derive(Clone)]
enum Act {
    Send(Command),
    AdjExcl(i32),
    AdjMargin(usize, i32), // 0=t 1=r 2=b 3=l
    AdjSize(i32, i32),
    AdjPopOff(i32, i32),
    AdjPopSize(i32, i32),
    AdjDpi(i32),
    CycleAnchor,
    CycleGravity,
    ToggleProto(u8), // 0 vp, 1 fs, 2 foreign-wlr, 3 foreign-ext
    Respawn,
}

struct Btn {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: String,
    act: Act,
}
fn btn(x: i32, y: i32, w: i32, label: impl Into<String>, act: Act) -> Btn {
    Btn { x, y, w, h: 20, label: label.into(), act }
}

/// All adjustable parameter state + the button/header layout it produces. Kept free of
/// Wayland handles so it can be exercised headlessly (see [`Params::layout`] and tests).
struct Params {
    excl: i32,
    margins: (i32, i32, i32, i32),
    size: (i32, i32),
    pop_off: (i32, i32),
    pop_size: (i32, i32),
    move_step: i32,
    anchor_i: usize,
    gravity_i: usize,
    dpi_num: i32,
    proto_vp: bool,
    proto_fs: bool,
    proto_fwlr: bool,
    proto_fext: bool,
}

impl Params {
    fn new() -> Self {
        Params {
            excl: 0,
            margins: (0, 0, 0, 0),
            size: (480, 320),
            pop_off: (0, 0),
            pop_size: (160, 120),
            move_step: 40,
            anchor_i: 7,  // BottomLeft
            gravity_i: 8, // BottomRight
            dpi_num: 2,
            proto_vp: true,
            proto_fs: true,
            proto_fwlr: true,
            proto_fext: true,
        }
    }

    /// Build the button set + header labels from current state. Pure function of state.
    fn layout(&self) -> (Vec<Btn>, Vec<(i32, i32, String)>) {
        let mut b: Vec<Btn> = Vec::new();
        let mut h: Vec<(i32, i32, String)> = Vec::new();

        let cw = 248;
        let half = (cw - 6) / 2;

        // ---- Column A ---------------------------------------------------------------
        let x = 8;
        let mut y = 8;
        let mut hdr = |h: &mut Vec<(i32, i32, String)>, y: &mut i32, t: &str| {
            h.push((x, *y, t.to_string()));
            *y += 16;
        };

        hdr(&mut h, &mut y, "LAYER");
        for (label, l) in [
            ("BACKGROUND", Layer::Background),
            ("BOTTOM", Layer::Bottom),
            ("TOP", Layer::Top),
            ("OVERLAY", Layer::Overlay),
        ] {
            let hw = (cw - 6) / 4;
            let i = match l {
                Layer::Background => 0,
                Layer::Bottom => 1,
                Layer::Top => 2,
                Layer::Overlay => 3,
            };
            b.push(btn(x + i * (hw + 2), y, hw, label, Act::Send(Command::SetLayer(l))));
        }
        y += 23;

        hdr(&mut h, &mut y, "ANCHOR");
        b.push(btn(x, y, 56, "TOP", Act::Send(Command::AnchorTop)));
        b.push(btn(x + 60, y, 56, "BOTTOM", Act::Send(Command::AnchorBottom)));
        b.push(btn(x + 120, y, 56, "LEFT", Act::Send(Command::AnchorLeft)));
        b.push(btn(x + 180, y, 56, "RIGHT", Act::Send(Command::AnchorRight)));
        y += 23;
        b.push(btn(x, y, half, "ANCHOR ALL", Act::Send(Command::AnchorAll)));
        b.push(btn(x + half + 6, y, half, "ANCHOR NONE", Act::Send(Command::AnchorNone)));
        y += 28;

        hdr(&mut h, &mut y, &format!("EXCLUSIVE ZONE  ({})", self.excl));
        b.push(btn(x, y, 56, "-1", Act::Send(Command::Excl(-1))));
        b.push(btn(x + 60, y, 56, "0", Act::Send(Command::Excl(0))));
        b.push(btn(x + 120, y, 56, "EZ -8", Act::AdjExcl(-8)));
        b.push(btn(x + 180, y, 56, "EZ +8", Act::AdjExcl(8)));
        y += 28;

        let (mt, mr, mb, ml) = self.margins;
        hdr(&mut h, &mut y, &format!("MARGIN  t{mt} r{mr} b{mb} l{ml}"));
        b.push(btn(x, y, 28, "T-", Act::AdjMargin(0, -4)));
        b.push(btn(x + 30, y, 28, "T+", Act::AdjMargin(0, 4)));
        b.push(btn(x + 62, y, 28, "R-", Act::AdjMargin(1, -4)));
        b.push(btn(x + 92, y, 28, "R+", Act::AdjMargin(1, 4)));
        b.push(btn(x + 124, y, 28, "B-", Act::AdjMargin(2, -4)));
        b.push(btn(x + 154, y, 28, "B+", Act::AdjMargin(2, 4)));
        b.push(btn(x + 186, y, 28, "L-", Act::AdjMargin(3, -4)));
        b.push(btn(x + 216, y, 28, "L+", Act::AdjMargin(3, 4)));
        y += 28;

        hdr(&mut h, &mut y, "KEYBOARD INTERACTIVITY");
        b.push(btn(x, y, 76, "NONE", Act::Send(Command::Keyboard(Kbd::None))));
        b.push(btn(x + 82, y, 76, "EXCLUSIVE", Act::Send(Command::Keyboard(Kbd::Exclusive))));
        b.push(btn(x + 164, y, 80, "ONDEMAND", Act::Send(Command::Keyboard(Kbd::OnDemand))));
        y += 28;

        hdr(&mut h, &mut y, &format!("SIZE  {}x{}", self.size.0, self.size.1));
        b.push(btn(x, y, 28, "W-", Act::AdjSize(-20, 0)));
        b.push(btn(x + 30, y, 28, "W+", Act::AdjSize(20, 0)));
        b.push(btn(x + 62, y, 28, "H-", Act::AdjSize(0, -20)));
        b.push(btn(x + 92, y, 28, "H+", Act::AdjSize(0, 20)));
        b.push(btn(x + 124, y, 120, "SIZE 0x0 STRETCH", Act::Send(Command::Size(0, 0))));

        // ---- Column B ---------------------------------------------------------------
        let x = 8 + cw + 12;
        let mut y = 8;
        hdr(&mut h, &mut y, "POPUP  (from layer surface)");
        b.push(btn(x, y, half, "POPUP ADD", Act::Send(Command::PopupAdd)));
        b.push(btn(x + half + 6, y, half, "POPUP CLOSE", Act::Send(Command::PopupClose)));
        y += 23;
        b.push(btn(x, y, half, &format!("ANCHOR {:?}", ANCHORS[self.anchor_i]), Act::CycleAnchor));
        b.push(btn(x + half + 6, y, half, &format!("GRAV {:?}", ANCHORS[self.gravity_i]), Act::CycleGravity));
        y += 23;
        h.push((x, y, format!("POPUP OFF {},{}", self.pop_off.0, self.pop_off.1)));
        y += 16;
        b.push(btn(x, y, 56, "OFFX-", Act::AdjPopOff(-40, 0)));
        b.push(btn(x + 60, y, 56, "OFFX+", Act::AdjPopOff(40, 0)));
        b.push(btn(x + 120, y, 56, "OFFY-", Act::AdjPopOff(0, -40)));
        b.push(btn(x + 180, y, 56, "OFFY+", Act::AdjPopOff(0, 40)));
        y += 23;
        h.push((x, y, format!("POPUP SIZE {}x{}", self.pop_size.0, self.pop_size.1)));
        y += 16;
        b.push(btn(x, y, 56, "W-", Act::AdjPopSize(-20, 0)));
        b.push(btn(x + 60, y, 56, "W+", Act::AdjPopSize(20, 0)));
        b.push(btn(x + 120, y, 56, "H-", Act::AdjPopSize(0, -20)));
        b.push(btn(x + 180, y, 56, "H+", Act::AdjPopSize(0, 20)));
        y += 23;
        h.push((x, y, format!("POPUP MOVE (step {})", self.move_step)));
        y += 16;
        b.push(btn(x, y, 56, "-X", Act::Send(Command::PopupMove(-self.move_step, 0))));
        b.push(btn(x + 60, y, 56, "+X", Act::Send(Command::PopupMove(self.move_step, 0))));
        b.push(btn(x + 120, y, 56, "-Y", Act::Send(Command::PopupMove(0, -self.move_step))));
        b.push(btn(x + 180, y, 56, "+Y", Act::Send(Command::PopupMove(0, self.move_step))));
        y += 28;

        hdr(&mut h, &mut y, "INPUT REGION");
        b.push(btn(x, y, 56, "FULL", Act::Send(Command::Input(Region::Full))));
        b.push(btn(x + 60, y, 56, "NONE", Act::Send(Command::Input(Region::None))));
        b.push(btn(x + 120, y, 56, "CIRCLE", Act::Send(Command::Input(Region::Circle))));
        b.push(btn(x + 180, y, 56, "HOLES", Act::Send(Command::Input(Region::Holes))));
        y += 28;

        hdr(&mut h, &mut y, "OPAQUE REGION");
        b.push(btn(x, y, half, "OPAQUE FULL", Act::Send(Command::Opaque(true))));
        b.push(btn(x + half + 6, y, half, "OPAQUE NONE", Act::Send(Command::Opaque(false))));
        y += 28;

        hdr(&mut h, &mut y, "OUTPUT / APPEARANCE");
        b.push(btn(x, y, cw, "OUTPUT CYCLE", Act::Send(Command::OutputCycle)));
        y += 23;
        b.push(btn(x, y, half, "TRANSP ON", Act::Send(Command::Transparent(true))));
        b.push(btn(x + half + 6, y, half, "TRANSP OFF", Act::Send(Command::Transparent(false))));
        y += 23;
        b.push(btn(x, y, half, "ANIM ON", Act::Send(Command::Animate(true))));
        b.push(btn(x + half + 6, y, half, "ANIM OFF", Act::Send(Command::Animate(false))));

        // ---- Column C ---------------------------------------------------------------
        let x = 8 + (cw + 12) * 2;
        let mut y = 8;
        hdr(&mut h, &mut y, &format!("BUFFER SCALE  (dpi={})", self.dpi_num));
        b.push(btn(x, y, half, "NORMAL", Act::Send(Command::Scale(ScaleReq::Normal))));
        b.push(btn(x + half + 6, y, half, "FS HONOR", Act::Send(Command::Scale(ScaleReq::FsHonor))));
        y += 23;
        b.push(btn(x, y, half, "FS IGNORE", Act::Send(Command::Scale(ScaleReq::FsIgnore))));
        b.push(btn(x + half + 6, y, half, "DPI HONOR", Act::Send(Command::Scale(ScaleReq::DpiHonor))));
        y += 23;
        b.push(btn(x, y, half, "DPI IGNORE", Act::Send(Command::Scale(ScaleReq::DpiIgnore))));
        b.push(btn(x + half + 6, y, 36, &format!("DPI {}", self.dpi_num), Act::Send(Command::Scale(ScaleReq::DpiScale(self.dpi_num)))));
        b.push(btn(x + half + 6 + 40, y, 20, "-", Act::AdjDpi(-1)));
        b.push(btn(x + half + 6 + 62, y, 20, "+", Act::AdjDpi(1)));
        y += 28;

        hdr(&mut h, &mut y, "FOREIGN TOPLEVEL (wlr control)");
        b.push(btn(x, y, half, "SEL PREV", Act::Send(Command::ForeignSel(-1))));
        b.push(btn(x + half + 6, y, half, "SEL NEXT", Act::Send(Command::ForeignSel(1))));
        y += 23;
        b.push(btn(x, y, half, "ACTIVATE", Act::Send(Command::Foreign(ForeignCtl::Activate))));
        b.push(btn(x + half + 6, y, half, "CLOSE", Act::Send(Command::Foreign(ForeignCtl::Close))));
        y += 23;
        b.push(btn(x, y, 56, "MAX+", Act::Send(Command::Foreign(ForeignCtl::Maximized(true)))));
        b.push(btn(x + 60, y, 56, "MAX-", Act::Send(Command::Foreign(ForeignCtl::Maximized(false)))));
        b.push(btn(x + 120, y, 56, "MIN+", Act::Send(Command::Foreign(ForeignCtl::Minimized(true)))));
        b.push(btn(x + 180, y, 56, "MIN-", Act::Send(Command::Foreign(ForeignCtl::Minimized(false)))));
        y += 23;
        b.push(btn(x, y, half, "FULLSCR ON", Act::Send(Command::Foreign(ForeignCtl::Fullscreen(true)))));
        b.push(btn(x + half + 6, y, half, "FULLSCR OFF", Act::Send(Command::Foreign(ForeignCtl::Fullscreen(false)))));
        y += 28;

        hdr(&mut h, &mut y, "LIFECYCLE");
        b.push(btn(x, y, half, "RECREATE", Act::Send(Command::Recreate)));
        b.push(btn(x + half + 6, y, half, "QUIT SUBJECT", Act::Send(Command::Quit)));
        y += 28;

        hdr(&mut h, &mut y, "PROTOCOLS (respawn to apply)");
        b.push(btn(x, y, 56, proto_label("VP", self.proto_vp), Act::ToggleProto(0)));
        b.push(btn(x + 60, y, 56, proto_label("FS", self.proto_fs), Act::ToggleProto(1)));
        b.push(btn(x + 120, y, 56, proto_label("FWLR", self.proto_fwlr), Act::ToggleProto(2)));
        b.push(btn(x + 180, y, 56, proto_label("FEXT", self.proto_fext), Act::ToggleProto(3)));
        y += 23;
        b.push(btn(x, y, cw, "RESPAWN SUBJECT", Act::Respawn));

        (b, h)
    }
}

struct Controller {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    pointer: Option<WlPointer>,

    width: i32,
    height: i32,
    first_configure: bool,
    exit: bool,
    ptr: Option<(f64, f64)>,
    need_redraw: bool,

    subject_path: std::path::PathBuf,
    child: Option<Child>,
    child_stdin: Option<ChildStdin>,

    p: Params,
    log: Vec<String>,
}

fn main() {
    diag::set_role("controller");

    if std::env::args().any(|a| a == "--selftest") {
        return selftest();
    }
    info!("controller starting");

    let subject_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("layer-stress-subject")))
        .unwrap_or_else(|| std::path::PathBuf::from("layer-stress-subject"));

    let conn = Connection::connect_to_env().expect("connect to wayland");
    let (globals, event_queue) = registry_queue_init::<Controller>(&conn).expect("registry init");
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<Controller> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .expect("insert wayland source");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    let pool = SlotPool::new((WIN_W * WIN_H * 4) as usize, &shm).expect("slot pool");

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("y5 layer-stress CONTROLLER");
    window.set_app_id("y5.layer.stress.controller");
    window.set_min_size(Some((WIN_W as u32, WIN_H as u32)));
    window.commit();

    let mut ctrl = Controller {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        window,
        pointer: None,
        width: WIN_W,
        height: WIN_H,
        first_configure: true,
        exit: false,
        ptr: None,
        need_redraw: false,
        subject_path,
        child: None,
        child_stdin: None,
        p: Params::new(),
        log: Vec::new(),
    };

    ctrl.spawn_subject();

    loop {
        event_loop.dispatch(std::time::Duration::from_millis(16), &mut ctrl).unwrap();
        if ctrl.exit {
            if let Some(mut c) = ctrl.child.take() {
                let _ = c.kill();
            }
            break;
        }
    }
}

/// Headless verification: enumerate the layout and confirm every `Send` command round-trips.
fn selftest() {
    let p = Params::new();
    let (buttons, headers) = p.layout();
    let mut sends = 0;
    for b in &buttons {
        if let Act::Send(cmd) = &b.act {
            sends += 1;
            let line = cmd.encode();
            assert_eq!(Command::parse(&line), Some(cmd.clone()), "round-trip failed: {line}");
            println!("  {:<18} -> {}", b.label, line);
        }
    }
    println!(
        "SELFTEST OK: {} buttons ({} send-commands), {} headers",
        buttons.len(),
        sends,
        headers.len()
    );
}

impl Controller {
    fn spawn_subject(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let mut cmd = PCommand::new(&self.subject_path);
        if !self.p.proto_vp {
            cmd.arg("--no-viewporter");
        }
        if !self.p.proto_fs {
            cmd.arg("--no-fractional-scale");
        }
        if !self.p.proto_fwlr {
            cmd.arg("--no-foreign-wlr");
        }
        if !self.p.proto_fext {
            cmd.arg("--no-foreign-ext");
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        match cmd.spawn() {
            Ok(mut child) => {
                self.child_stdin = child.stdin.take();
                self.child = Some(child);
                self.push_log(format!(
                    "spawned subject (vp={} fs={} fwlr={} fext={})",
                    self.p.proto_vp, self.p.proto_fs, self.p.proto_fwlr, self.p.proto_fext
                ));
                info!("spawned subject: {}", self.subject_path.display());
            }
            Err(e) => {
                warn!("failed to spawn subject {}: {e}", self.subject_path.display());
                self.push_log(format!("SPAWN FAILED: {e}"));
            }
        }
    }

    fn send(&mut self, cmd: Command) {
        let line = cmd.encode();
        if let Some(stdin) = &mut self.child_stdin {
            if writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_ok() {
                self.push_log(format!("> {line}"));
            } else {
                self.push_log("> (subject pipe closed)".into());
            }
        } else {
            self.push_log("> (no subject)".into());
        }
    }

    fn push_log(&mut self, s: String) {
        self.log.push(s);
        let len = self.log.len();
        if len > 12 {
            self.log.drain(0..len - 12);
        }
    }

    fn act(&mut self, a: Act) {
        match a {
            Act::Send(c) => self.send(c),
            Act::AdjExcl(d) => {
                self.p.excl += d;
                self.send(Command::Excl(self.p.excl));
            }
            Act::AdjMargin(i, d) => {
                let m = &mut self.p.margins;
                let slot = match i {
                    0 => &mut m.0,
                    1 => &mut m.1,
                    2 => &mut m.2,
                    _ => &mut m.3,
                };
                *slot = (*slot + d).max(0);
                let (t, r, b, l) = self.p.margins;
                self.send(Command::Margin(t, r, b, l));
            }
            Act::AdjSize(dw, dh) => {
                self.p.size = ((self.p.size.0 + dw).max(0), (self.p.size.1 + dh).max(0));
                self.send(Command::Size(self.p.size.0 as u32, self.p.size.1 as u32));
            }
            Act::AdjPopOff(dx, dy) => {
                self.p.pop_off = (self.p.pop_off.0 + dx, self.p.pop_off.1 + dy);
                self.send(Command::PopupOff(self.p.pop_off.0, self.p.pop_off.1));
            }
            Act::AdjPopSize(dw, dh) => {
                self.p.pop_size = ((self.p.pop_size.0 + dw).max(8), (self.p.pop_size.1 + dh).max(8));
                self.send(Command::PopupSize(self.p.pop_size.0 as u32, self.p.pop_size.1 as u32));
            }
            Act::AdjDpi(d) => self.p.dpi_num = (self.p.dpi_num + d).clamp(1, 8),
            Act::CycleAnchor => {
                self.p.anchor_i = (self.p.anchor_i + 1) % ANCHORS.len();
                self.send(Command::PopupAnchor(ANCHORS[self.p.anchor_i]));
            }
            Act::CycleGravity => {
                self.p.gravity_i = (self.p.gravity_i + 1) % ANCHORS.len();
                self.send(Command::PopupGravity(ANCHORS[self.p.gravity_i]));
            }
            Act::ToggleProto(i) => {
                match i {
                    0 => self.p.proto_vp = !self.p.proto_vp,
                    1 => self.p.proto_fs = !self.p.proto_fs,
                    2 => self.p.proto_fwlr = !self.p.proto_fwlr,
                    _ => self.p.proto_fext = !self.p.proto_fext,
                }
                self.push_log("proto toggled — press RESPAWN to apply".into());
            }
            Act::Respawn => self.spawn_subject(),
        }
        self.need_redraw = true;
    }

    fn on_click(&mut self, x: f64, y: f64) {
        let (x, y) = (x as i32, y as i32);
        let buttons = self.p.layout().0;
        if let Some(b) =
            buttons.into_iter().find(|b| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h)
        {
            self.act(b.act);
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let (w, hgt) = (self.width.max(1), self.height.max(1));
        let (buttons, headers) = self.p.layout();
        let ptr = self.ptr;
        let log = self.log.clone();
        let stride = w * 4;
        let (buffer, slice) =
            self.pool.create_buffer(w, hgt, stride, Format::Argb8888).expect("ctrl buffer");
        {
            let mut cv = Canvas::new(slice, w, hgt);
            cv.clear(color::PANEL);
            for (hx, hy, t) in &headers {
                font::text(&mut cv, *hx, *hy, 1, color::CYAN, t);
            }
            for bt in &buttons {
                let hot = ptr
                    .map(|(px, py)| {
                        let (px, py) = (px as i32, py as i32);
                        px >= bt.x && px < bt.x + bt.w && py >= bt.y && py < bt.y + bt.h
                    })
                    .unwrap_or(false);
                cv.rect(bt.x, bt.y, bt.w, bt.h, if hot { color::BTN_HOT } else { color::BTN });
                cv.frame(bt.x, bt.y, bt.w, bt.h, 1, color::DKGREY);
                font::text(&mut cv, bt.x + 4, bt.y + 6, 1, color::WHITE, &bt.label);
            }
            let log_y = hgt - 116;
            cv.rect(0, log_y - 4, w, hgt - (log_y - 4), 0xFF14181C);
            font::text(&mut cv, 8, log_y, 1, color::YELLOW, "COMMAND LOG");
            let mut ly = log_y + 14;
            for line in &log {
                font::text(&mut cv, 8, ly, 1, color::LTGREY, line);
                ly += 9;
            }
        }
        self.window.wl_surface().damage_buffer(0, 0, w, hgt);
        buffer.attach_to(self.window.wl_surface()).expect("attach");
        self.window.commit();
        let _ = qh;
    }
}

fn proto_label(name: &str, on: bool) -> String {
    format!("{name} {}", if on { "+" } else { "-" })
}

// ==========================================================================================
// sctk handlers
// ==========================================================================================

impl CompositorHandler for Controller {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &WlSurface, _: u32) {
        self.draw(qh);
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
}

impl WindowHandler for Controller {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }
    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
        self.width = configure.new_size.0.map(|v| v.get() as i32).unwrap_or(WIN_W);
        self.height = configure.new_size.1.map(|v| v.get() as i32).unwrap_or(WIN_H);
        if self.first_configure {
            self.first_configure = false;
        }
        self.draw(qh);
    }
}

impl OutputHandler for Controller {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Controller {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl SeatHandler for Controller {
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

impl PointerHandler for Controller {
    fn pointer_frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &WlPointer, events: &[PointerEvent]) {
        let mut clicked: Option<(f64, f64)> = None;
        for e in events {
            if &e.surface != self.window.wl_surface() {
                continue;
            }
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.ptr = Some(e.position);
                    self.need_redraw = true;
                }
                PointerEventKind::Leave { .. } => {
                    self.ptr = None;
                    self.need_redraw = true;
                }
                PointerEventKind::Press { button, .. } if button == 0x110 => {
                    clicked = Some(e.position);
                }
                _ => {}
            }
        }
        if let Some((x, y)) = clicked {
            self.on_click(x, y);
        }
        if self.need_redraw {
            self.need_redraw = false;
            self.draw(qh);
        }
    }
}

impl ProvidesRegistryState for Controller {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Controller);
delegate_output!(Controller);
delegate_shm!(Controller);
delegate_seat!(Controller);
delegate_pointer!(Controller);
delegate_xdg_shell!(Controller);
delegate_xdg_window!(Controller);
delegate_registry!(Controller);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_commands_round_trip() {
        let p = Params::new();
        let (buttons, _) = p.layout();
        assert!(buttons.len() > 50, "expected a rich button grid, got {}", buttons.len());
        let mut sends = 0;
        for b in &buttons {
            if let Act::Send(cmd) = &b.act {
                sends += 1;
                let line = cmd.encode();
                assert_eq!(Command::parse(&line), Some(cmd.clone()), "round-trip: {line}");
            }
        }
        assert!(sends > 30, "expected many send-commands, got {sends}");
    }
}
