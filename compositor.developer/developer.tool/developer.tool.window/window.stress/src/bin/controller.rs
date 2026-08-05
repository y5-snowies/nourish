//! The controller: a well-behaved GUI window that spawns the subject as a child process and
//! drives it by writing [`Command`] lines to its stdin. Clickable buttons, grouped by
//! scenario, plus +/- steppers for parameters and a command-log panel.
//!
//! The subject binary is found next to this executable (`window-stress-subject`).
//!
//! Run with `--selftest` to enumerate the button/command surface without connecting to
//! Wayland or spawning the subject (used for headless verification).

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

use window_stress::canvas::{Canvas, color};
use window_stress::diag;
use window_stress::protocol::{Anchor, Command, DecoMode};
use window_stress::{font, info, warn};

const WIN_W: i32 = 780;
const WIN_H: i32 = 760;
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
const PALETTE: [(u8, u8, u8); 7] = [
    (224, 64, 64),
    (64, 192, 64),
    (64, 96, 224),
    (240, 240, 240),
    (224, 192, 64),
    (64, 192, 224),
    (224, 64, 192),
];

/// A click action: either a command to forward to the subject, or a controller-side action.
#[derive(Clone)]
enum Act {
    Send(Command),
    AdjDelta(i32),
    AdjOff(i32, i32),
    AdjPopSize(i32, i32),
    AdjDest(i32, i32),
    AdjFs(i32),
    AdjDpi(i32),
    CycleAnchor,
    CycleGravity,
    CycleColor,
    ToggleProto(u8), // 0 deco, 1 vp, 2 fs, 3 sp
    Respawn,
    /// Spawn N extra DETACHED session subjects (see `Controller::spawn_session`).
    SpawnSession(u32),
    /// Forget the spawned session subjects (they outlive the controller by design).
    ForgetSession,
    /// Wipe the subjects' id→value store so the next run starts from NEW.
    ClearSessionStore,
    /// Flip which session-management namespace the detached subjects bind.
    ToggleSessionNs,
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
    delta: i32,
    off: (i32, i32),
    move_step: i32,
    pop_size: (i32, i32),
    dest: (i32, i32),
    anchor_i: usize,
    gravity_i: usize,
    fs_num: u32,
    dpi_num: i32,
    color_i: usize,
    proto_deco: bool,
    proto_vp: bool,
    proto_fs: bool,
    proto_sp: bool,
    /// false = xdg_session_manager_v1 (current staging name),
    /// true  = xx_session_manager_v1 (pre-rename; what GTK 4.22 binds).
    session_xx: bool,
}

impl Params {
    fn new() -> Self {
        Params {
            delta: 40,
            off: (0, 0),
            move_step: 40,
            pop_size: (160, 120),
            dest: (480, 320),
            anchor_i: 7,  // BottomLeft
            gravity_i: 8, // BottomRight
            fs_num: 180,
            dpi_num: 2,
            color_i: 0,
            proto_deco: true,
            proto_vp: true,
            proto_fs: true,
            proto_sp: true,
            session_xx: false,
        }
    }

    fn color_rgba(&self) -> (u8, u8, u8, u8) {
        let (r, g, b) = PALETTE[self.color_i % PALETTE.len()];
        (r, g, b, 255)
    }

    /// Build the button set and header labels from current state. Pure function of state, so
    /// click hit-testing and drawing stay consistent.
    fn layout(&self) -> (Vec<Btn>, Vec<(i32, i32, String)>) {
        let mut b: Vec<Btn> = Vec::new();
        let mut h: Vec<(i32, i32, String)> = Vec::new();

        let cw = 240;
        let half = (cw - 6) / 2;

        // Column A
        let x = 8;
        let mut y = 8;
        let mut hdr = |h: &mut Vec<(i32, i32, String)>, y: &mut i32, t: &str| {
            h.push((x, *y, t.to_string()));
            *y += 16;
        };

        hdr(&mut h, &mut y, "DECORATION");
        for (label, m) in [
            ("DECO SERVER", DecoMode::Server),
            ("DECO CLIENT", DecoMode::Client),
            ("DECO NONE", DecoMode::None),
        ] {
            b.push(btn(x, y, cw, label, Act::Send(Command::DecoMode(m))));
            y += 23;
        }
        b.push(btn(x, y, half, "DECO IGNORE", Act::Send(Command::DecoIgnore)));
        b.push(btn(x + half + 6, y, half, "DECO BADSIZE", Act::Send(Command::DecoBadsize)));
        y += 28;

        hdr(&mut h, &mut y, &format!("BUFFER  (delta={})", self.delta));
        b.push(btn(x, y, cw, "BUF AGREED", Act::Send(Command::BufAgreed)));
        y += 23;
        b.push(btn(x, y, half, &format!("BUF DELTA {}", self.delta), Act::Send(Command::BufDelta(self.delta))));
        b.push(btn(x + half + 6, y, 36, "-8", Act::AdjDelta(-8)));
        b.push(btn(x + half + 6 + 40, y, 36, "+8", Act::AdjDelta(8)));
        y += 23;
        b.push(btn(x, y, half, "BUF ZERO", Act::Send(Command::BufZero)));
        b.push(btn(x + half + 6, y, half, "BUF NOACK", Act::Send(Command::BufNoack)));
        y += 23;
        b.push(btn(x, y, half, "BUF PREACK", Act::Send(Command::BufPreack)));
        b.push(btn(x + half + 6, y, half, "GEO MISMATCH", Act::Send(Command::GeoMismatch)));
        y += 28;

        hdr(&mut h, &mut y, "LIFECYCLE");
        b.push(btn(x, y, half, "MAP", Act::Send(Command::Map)));
        b.push(btn(x + half + 6, y, half, "UNMAP", Act::Send(Command::Unmap)));
        y += 23;
        b.push(btn(x, y, half, "MAPCYCLE ON", Act::Send(Command::MapCycle(true))));
        b.push(btn(x + half + 6, y, half, "MAPCYCLE OFF", Act::Send(Command::MapCycle(false))));
        y += 23;
        b.push(btn(x, y, cw, "SIZE 480x320", Act::Send(Command::Size(480, 320))));
        y += 23;
        b.push(btn(x, y, cw, "QUIT SUBJECT", Act::Send(Command::Quit)));
        y += 30;

        // TEARING — the pacing test. The hint tags this client as a pacer for the
        // compositor's Exclusive mode; the rate buttons drive commits off the
        // subject's own timer so the compositor's flip rate is an independent
        // measurement rather than an echo of the cadence it already granted.
        //
        // The decisive check needs a SECOND, untagged subject: flip rate must
        // track the tagged client and stay INVARIANT to the untagged one. With
        // one window, Exclusive and Always are indistinguishable.
        hdr(&mut h, &mut y, "TEARING");
        b.push(btn(x, y, half, "HINT ASYNC", Act::Send(Command::Tearing(true))));
        b.push(btn(x + half + 6, y, half, "HINT VSYNC", Act::Send(Command::Tearing(false))));
        y += 23;
        let q = (cw - 18) / 4;
        for (i, hz) in [0u32, 30, 60, 120].iter().enumerate() {
            let label = if *hz == 0 { "OFF".to_string() } else { format!("{hz}Hz") };
            b.push(btn(x + (q + 6) * i as i32, y, q, &label, Act::Send(Command::CommitRate(*hz))));
        }
        y += 23;
        for (i, hz) in [144u32, 240, 300, 500].iter().enumerate() {
            b.push(btn(x + (q + 6) * i as i32, y, q, &format!("{hz}Hz"), Act::Send(Command::CommitRate(*hz))));
        }
        y += 23;
        b.push(btn(x, y, half, "COUNTER ON", Act::Send(Command::ShowCounter(true))));
        b.push(btn(x + half + 6, y, half, "COUNTER OFF", Act::Send(Command::ShowCounter(false))));

        // Column B
        let x = 8 + cw + 12;
        let mut y = 8;
        hdr(&mut h, &mut y, "POPUP");
        b.push(btn(x, y, half, "POPUP ADD", Act::Send(Command::PopupAdd)));
        b.push(btn(x + half + 6, y, half, "POPUP NEST", Act::Send(Command::PopupNest)));
        y += 23;
        b.push(btn(x, y, half, "POPUP CLOSE", Act::Send(Command::PopupClose)));
        b.push(btn(x + half + 6, y, half, &format!("ANCHOR {:?}", ANCHORS[self.anchor_i]), Act::CycleAnchor));
        y += 23;
        b.push(btn(x, y, cw, &format!("GRAVITY {:?}", ANCHORS[self.gravity_i]), Act::CycleGravity));
        y += 23;
        h.push((x, y, format!("OFF {},{}", self.off.0, self.off.1)));
        y += 16;
        b.push(btn(x, y, 56, "OFFX-", Act::AdjOff(-40, 0)));
        b.push(btn(x + 60, y, 56, "OFFX+", Act::AdjOff(40, 0)));
        b.push(btn(x + 120, y, 56, "OFFY-", Act::AdjOff(0, -40)));
        b.push(btn(x + 180, y, 56, "OFFY+", Act::AdjOff(0, 40)));
        y += 23;
        h.push((x, y, format!("POPSIZE {}x{}", self.pop_size.0, self.pop_size.1)));
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

        hdr(&mut h, &mut y, "SUBSURFACE");
        b.push(btn(x, y, half, "SUB ADD", Act::Send(Command::SubAdd)));
        b.push(btn(x + half + 6, y, half, "SUB NEST", Act::Send(Command::SubNest)));
        y += 23;
        b.push(btn(x, y, half, "SUB REMOVE", Act::Send(Command::SubRemove)));
        b.push(btn(x + half + 6, y, half, "SUB SYNC", Act::Send(Command::SubSync)));
        y += 23;
        b.push(btn(x, y, cw, "SUB DESYNC", Act::Send(Command::SubDesync)));
        y += 23;
        h.push((x, y, "SUB MOVE".to_string()));
        y += 16;
        b.push(btn(x, y, 56, "-X", Act::Send(Command::SubMove(-self.move_step, 0))));
        b.push(btn(x + 60, y, 56, "+X", Act::Send(Command::SubMove(self.move_step, 0))));
        b.push(btn(x + 120, y, 56, "-Y", Act::Send(Command::SubMove(0, -self.move_step))));
        b.push(btn(x + 180, y, 56, "+Y", Act::Send(Command::SubMove(0, self.move_step))));

        // Column C
        let x = 8 + (cw + 12) * 2;
        let mut y = 8;
        hdr(&mut h, &mut y, &format!("VIEWPORTER  dest={}x{}", self.dest.0, self.dest.1));
        b.push(btn(x, y, half, "VP DEST", Act::Send(Command::VpDest(self.dest.0, self.dest.1))));
        b.push(btn(x + half + 6, y, half, "VP UNSET", Act::Send(Command::VpUnset)));
        y += 23;
        b.push(btn(x, y, 56, "W-", Act::AdjDest(-20, 0)));
        b.push(btn(x + 60, y, 56, "W+", Act::AdjDest(20, 0)));
        b.push(btn(x + 120, y, 56, "H-", Act::AdjDest(0, -20)));
        b.push(btn(x + 180, y, 56, "H+", Act::AdjDest(0, 20)));
        y += 23;
        b.push(btn(x, y, half, "VP DELTA -20", Act::Send(Command::VpDestDelta(-20))));
        b.push(btn(x + half + 6, y, half, "VP DELTA +20", Act::Send(Command::VpDestDelta(20))));
        y += 23;
        b.push(btn(x, y, half, "VP SRC CROP", Act::Send(Command::VpSrc(0.0, 0.0, 160.0, 120.0))));
        b.push(btn(x + half + 6, y, half, "VP BAD", Act::Send(Command::VpBad)));
        y += 23;
        b.push(btn(x, y, half, "VP ANIM ON", Act::Send(Command::VpAnimate(true))));
        b.push(btn(x + half + 6, y, half, "VP ANIM OFF", Act::Send(Command::VpAnimate(false))));
        y += 28;

        hdr(&mut h, &mut y, &format!("FRACTIONAL  scale={}/120", self.fs_num));
        b.push(btn(x, y, 56, "FS HON", Act::Send(Command::FsHonor)));
        b.push(btn(x + 60, y, 56, "FS IGN", Act::Send(Command::FsIgnore)));
        b.push(btn(x + 120, y, 56, "NOVP", Act::Send(Command::FsNoViewport)));
        b.push(btn(x + 180, y, 56, "MISM", Act::Send(Command::FsMismatch)));
        y += 23;
        b.push(btn(x, y, half, &format!("FS SCALE {}", self.fs_num), Act::Send(Command::FsScale(self.fs_num))));
        b.push(btn(x + half + 6, y, 36, "-", Act::AdjFs(-30)));
        b.push(btn(x + half + 6 + 40, y, 36, "+", Act::AdjFs(30)));
        y += 28;

        hdr(&mut h, &mut y, &format!("DPI / INTEGER  scale={}", self.dpi_num));
        b.push(btn(x, y, 56, "HONOR", Act::Send(Command::DpiHonor)));
        b.push(btn(x + 60, y, 56, "IGNORE", Act::Send(Command::DpiIgnore)));
        b.push(btn(x + 120, y, 56, "NONDIV", Act::Send(Command::DpiNondiv)));
        b.push(btn(x + 180, y, 56, "MISM", Act::Send(Command::DpiMismatch)));
        y += 23;
        b.push(btn(x, y, half, &format!("DPI SCALE {}", self.dpi_num), Act::Send(Command::DpiScale(self.dpi_num))));
        b.push(btn(x + half + 6, y, 36, "-", Act::AdjDpi(-1)));
        b.push(btn(x + half + 6 + 40, y, 36, "+", Act::AdjDpi(1)));
        y += 23;
        b.push(btn(x, y, cw, "DPI ZERO (proto err)", Act::Send(Command::DpiZero)));
        y += 28;

        let (r, g, bb, a) = self.color_rgba();
        hdr(&mut h, &mut y, &format!("SINGLE-PIXEL  rgb=({r},{g},{bb})"));
        b.push(btn(x, y, half, "SP FILL", Act::Send(Command::SpFill(r, g, bb, a))));
        b.push(btn(x + half + 6, y, half, "SP SUB", Act::Send(Command::SpSub(r, g, bb, a))));
        y += 23;
        b.push(btn(x, y, half, "SP NOVP", Act::Send(Command::SpNoViewport)));
        b.push(btn(x + half + 6, y, half, "CYCLE COLOR", Act::CycleColor));
        y += 28;

        hdr(&mut h, &mut y, "PROTOCOLS (respawn to apply)");
        b.push(btn(x, y, 56, proto_label("DEC", self.proto_deco), Act::ToggleProto(0)));
        b.push(btn(x + 60, y, 56, proto_label("VP", self.proto_vp), Act::ToggleProto(1)));
        b.push(btn(x + 120, y, 56, proto_label("FS", self.proto_fs), Act::ToggleProto(2)));
        b.push(btn(x + 180, y, 56, proto_label("SP", self.proto_sp), Act::ToggleProto(3)));
        y += 23;
        b.push(btn(x, y, cw, "RESPAWN SUBJECT", Act::Respawn));
        y += 28;

        // The placeholder session-restore check. These subjects are spawned
        // DETACHED (no stdin pipe, no controller commands) precisely because the
        // point is what survives without a controller: close them in the
        // compositor, then press Launch on the placeholder tiles they leave
        // behind. Each should come back showing RESTORED and the same VALUE —
        // a value that is in none of the launch args the placeholder replays.
        hdr(&mut h, &mut y, "SESSION RESTORE (detached subjects)");
        b.push(btn(x, y, half, "SPAWN 1", Act::SpawnSession(1)));
        b.push(btn(x + half + 6, y, half, "SPAWN 3", Act::SpawnSession(3)));
        y += 23;
        b.push(btn(x, y, half, "FORGET", Act::ForgetSession));
        b.push(btn(x + half + 6, y, half, "CLEAR STORE", Act::ClearSessionStore));
        y += 23;
        b.push(btn(x, y, cw,
                   if self.session_xx { "NS: xx_ (GTK 4.22)" } else { "NS: xdg_ (staging)" },
                   Act::ToggleSessionNs));

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
    /// Detached session subjects (see `Act::SpawnSession`). Held only so they can
    /// be reaped; they are NOT killed on controller exit — the whole test is what
    /// happens to them after the controller is out of the picture.
    session_children: Vec<Child>,
    /// Controller-side log counter for detached spawns. Never reaches the
    /// subject — see `spawn_session` for why they must be argv-identical.
    next_session_tag: u32,

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
        .and_then(|p| p.parent().map(|d| d.join("window-stress-subject")))
        .unwrap_or_else(|| std::path::PathBuf::from("window-stress-subject"));

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
    window.set_title("y5 window-stress CONTROLLER");
    window.set_app_id("y5.window.stress.controller");
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
        session_children: Vec::new(),
        next_session_tag: 1,
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

/// Headless verification: enumerate the layout and confirm every `Send` command round-trips
/// through the wire encoding. No Wayland, no child process.
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
        if !self.p.proto_deco {
            cmd.arg("--no-decoration");
        }
        if !self.p.proto_vp {
            cmd.arg("--no-viewporter");
        }
        if !self.p.proto_fs {
            cmd.arg("--no-fractional-scale");
        }
        if !self.p.proto_sp {
            cmd.arg("--no-single-pixel");
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        match cmd.spawn() {
            Ok(mut child) => {
                self.child_stdin = child.stdin.take();
                self.child = Some(child);
                self.push_log(format!(
                    "spawned subject (deco={} vp={} fs={} sp={})",
                    self.p.proto_deco, self.p.proto_vp, self.p.proto_fs, self.p.proto_sp
                ));
                info!("spawned subject: {}", self.subject_path.display());
            }
            Err(e) => {
                warn!("failed to spawn subject {}: {e}", self.subject_path.display());
                self.push_log(format!("SPAWN FAILED: {e}"));
            }
        }
    }

    /// Spawn `n` DETACHED session subjects.
    ///
    /// `--detached` (so stdin EOF does not kill them) and stdin/stdout on null
    /// (so there is no controller pipe at all). That single flag is the ENTIRE
    /// command line: every subject is spawned byte-identical, with no index, no
    /// tag, no per-instance path.
    ///
    /// That is the point. A placeholder relaunches by replaying the argv it
    /// captured, so any stable discriminator here would be an alternative
    /// explanation for a value coming back — and one you could only rule out by
    /// reading the subject's source. With argv identical, the session id the
    /// compositor mints per placeholder is the only thing distinguishing them.
    fn spawn_session(&mut self, n: u32) {
        for _ in 0..n {
            let nth = self.next_session_tag;
            self.next_session_tag += 1;
            let mut cmd = PCommand::new(&self.subject_path);
            cmd.arg("--detached");
            if self.p.session_xx {
                cmd.arg("--xx");
            }
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
            match cmd.spawn() {
                Ok(child) => {
                    // `nth` is a controller-side log counter only — it is never
                    // passed to the subject.
                    info!("spawned detached session subject ({nth})");
                    let ns = if self.p.session_xx { "xx_" } else { "xdg_" };
                    self.push_log(format!("session subject {nth} up ({ns}, identical argv)"));
                    self.session_children.push(child);
                }
                Err(e) => {
                    warn!("failed to spawn session subject: {e}");
                    self.push_log(format!("SESSION SPAWN FAILED: {e}"));
                }
            }
        }
    }

    /// Stop tracking the detached subjects without killing them — closing them
    /// from inside the compositor is what produces the placeholder tiles.
    fn forget_session(&mut self) {
        let n = self.session_children.len();
        self.session_children.clear();
        self.push_log(format!("forgot {n} session subject(s) (still running)"));
    }

    /// Delete the shared id→value store, so the next spawn reports NEW rather
    /// than RESTORED. The control case for the check.
    fn clear_session_store(&mut self) {
        let path = window_stress::session::default_store();
        match std::fs::remove_file(&path) {
            Ok(()) => self.push_log(format!("cleared {}", path.display())),
            Err(e) => self.push_log(format!("clear store: {e}")),
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
        if len > 14 {
            self.log.drain(0..len - 14);
        }
    }

    fn act(&mut self, a: Act) {
        match a {
            Act::Send(c) => self.send(c),
            Act::AdjDelta(d) => self.p.delta += d,
            Act::AdjOff(dx, dy) => {
                self.p.off = (self.p.off.0 + dx, self.p.off.1 + dy);
                self.send(Command::PopupOff(self.p.off.0, self.p.off.1));
            }
            Act::AdjPopSize(dw, dh) => {
                self.p.pop_size = ((self.p.pop_size.0 + dw).max(8), (self.p.pop_size.1 + dh).max(8));
                self.send(Command::PopupSize(self.p.pop_size.0 as u32, self.p.pop_size.1 as u32));
            }
            Act::AdjDest(dw, dh) => {
                self.p.dest = ((self.p.dest.0 + dw).max(1), (self.p.dest.1 + dh).max(1));
                self.send(Command::VpDest(self.p.dest.0, self.p.dest.1));
            }
            Act::AdjFs(d) => self.p.fs_num = (self.p.fs_num as i32 + d).clamp(30, 480) as u32,
            Act::AdjDpi(d) => self.p.dpi_num = (self.p.dpi_num + d).clamp(1, 8),
            Act::CycleAnchor => {
                self.p.anchor_i = (self.p.anchor_i + 1) % ANCHORS.len();
                self.send(Command::PopupAnchor(ANCHORS[self.p.anchor_i]));
            }
            Act::CycleGravity => {
                self.p.gravity_i = (self.p.gravity_i + 1) % ANCHORS.len();
                self.send(Command::PopupGravity(ANCHORS[self.p.gravity_i]));
            }
            Act::CycleColor => self.p.color_i = (self.p.color_i + 1) % PALETTE.len(),
            Act::ToggleProto(i) => {
                match i {
                    0 => self.p.proto_deco = !self.p.proto_deco,
                    1 => self.p.proto_vp = !self.p.proto_vp,
                    2 => self.p.proto_fs = !self.p.proto_fs,
                    _ => self.p.proto_sp = !self.p.proto_sp,
                }
                self.push_log("proto toggled — press RESPAWN to apply".into());
            }
            Act::Respawn => self.spawn_subject(),
            Act::SpawnSession(n) => self.spawn_session(n),
            Act::ForgetSession => self.forget_session(),
            Act::ClearSessionStore => self.clear_session_store(),
            Act::ToggleSessionNs => {
                self.p.session_xx = !self.p.session_xx;
                let ns = if self.p.session_xx { "xx_" } else { "xdg_" };
                self.push_log(format!("session namespace -> {ns} (applies to next spawn)"));
            }
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
            // Command log panel along the bottom.
            let log_y = hgt - 130;
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
        assert!(sends > 40, "expected many send-commands, got {sends}");
    }
}
