//! The X11 client under test.
//!
//! Reads one [`Command`] per stdin line and does the corresponding X11 thing, then
//! reports what the SERVER thinks happened. It never speaks wayland: everything it
//! does reaches y5 through Xwayland and the in-process XWM, which is the path under
//! test.
//!
//! Built on `x11rb` — the same binding smithay's own XWM uses — so the requests it
//! sends are exactly the shape the compositor is written against.
//!
//! Windows are addressed by a small harness id (1, 2, 3 …) rather than by X11 window
//! id, because the controller has to be able to name a window in a script before the
//! server has told anyone what its real id is.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::mpsc;

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use x11_stress::protocol::{CloseMode, Command, IconSpec, InputModel, WmType};

/// Distinct fills so a screenshot is readable. Managed windows walk the first row;
/// override-redirect windows are deliberately paler, so "which of these is the menu"
/// needs no counting.
const MANAGED: [u32; 6] = [0x2f4858, 0x33658a, 0x86bbd8, 0x758e4f, 0xf6ae2d, 0xf26419];
const OVERRIDE: [u32; 4] = [0xd8e2dc, 0xffe5d9, 0xffcad4, 0xf4acb7];

struct Atoms {
    wm_protocols: Atom,
    wm_delete_window: Atom,
    wm_take_focus: Atom,
    net_wm_pid: Atom,
    net_wm_name: Atom,
    net_wm_state: Atom,
    net_wm_state_fullscreen: Atom,
    net_wm_state_modal: Atom,
    net_wm_state_skip_taskbar: Atom,
    net_wm_window_type: Atom,
    net_wm_icon: Atom,
    net_wm_window_opacity: Atom,
    xdnd_aware: Atom,
    xdnd_selection: Atom,
    xdnd_enter: Atom,
    xdnd_position: Atom,
    xdnd_status: Atom,
    xdnd_leave: Atom,
    xdnd_drop: Atom,
    xdnd_finished: Atom,
    xdnd_action_copy: Atom,
    xdnd_type_list: Atom,
    text_plain: Atom,
    /// One per [`WmType`] with an atom name, in `WmType::all()` order. A map rather
    /// than named fields because the harness sets them by name and reads them back
    /// for `info`.
    window_types: Vec<(WmType, Atom)>,
    utf8_string: Atom,
    clipboard: Atom,
    targets: Atom,
    stress_sel: Atom,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Self, Box<dyn std::error::Error>> {
        let mut one = |name: &str| -> Result<Atom, Box<dyn std::error::Error>> {
            Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
        };
        Ok(Self {
            wm_protocols: one("WM_PROTOCOLS")?,
            wm_delete_window: one("WM_DELETE_WINDOW")?,
            wm_take_focus: one("WM_TAKE_FOCUS")?,
            net_wm_pid: one("_NET_WM_PID")?,
            net_wm_name: one("_NET_WM_NAME")?,
            net_wm_state: one("_NET_WM_STATE")?,
            net_wm_state_fullscreen: one("_NET_WM_STATE_FULLSCREEN")?,
            net_wm_state_modal: one("_NET_WM_STATE_MODAL")?,
            net_wm_state_skip_taskbar: one("_NET_WM_STATE_SKIP_TASKBAR")?,
            net_wm_window_type: one("_NET_WM_WINDOW_TYPE")?,
            net_wm_icon: one("_NET_WM_ICON")?,
            net_wm_window_opacity: one("_NET_WM_WINDOW_OPACITY")?,
            xdnd_aware: one("XdndAware")?,
            xdnd_selection: one("XdndSelection")?,
            xdnd_enter: one("XdndEnter")?,
            xdnd_position: one("XdndPosition")?,
            xdnd_status: one("XdndStatus")?,
            xdnd_leave: one("XdndLeave")?,
            xdnd_drop: one("XdndDrop")?,
            xdnd_finished: one("XdndFinished")?,
            xdnd_action_copy: one("XdndActionCopy")?,
            xdnd_type_list: one("XdndTypeList")?,
            text_plain: one("text/plain;charset=utf-8")?,
            window_types: {
                let mut v = Vec::new();
                for t in WmType::all() {
                    if let Some(name) = t.atom_name() {
                        v.push((t, one(name)?));
                    }
                }
                v
            },
            utf8_string: one("UTF8_STRING")?,
            clipboard: one("CLIPBOARD")?,
            targets: one("TARGETS")?,
            stress_sel: one("Y5_X11_STRESS_SEL")?,
        })
    }
}

/// Everything that decides what a new window IS, in one place — because y5 reads all
/// of it at the map request and the answer depends on the combination, not on any one
/// field. An override-redirect child with a `menu` type and a resolvable
/// `WM_TRANSIENT_FOR` is a popup; drop any one of the three and it is a window.
struct Spec {
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    override_redirect: bool,
    transient_for: Option<Window>,
    wm_type: WmType,
    /// Create on a 32-bit ARGB visual instead of the screen's default. See
    /// [`Command::MapArgb`] — this is what gives the window a non-opaque region.
    argb: bool,
}

impl Spec {
    fn managed(x: i16, y: i16, w: u16, h: u16) -> Self {
        Self {
            x, y, w, h,
            override_redirect: false,
            transient_for: None,
            wm_type: WmType::Unset,
            argb: false,
        }
    }
    fn argb(mut self) -> Self {
        self.argb = true;
        self
    }
    fn override_redirect(mut self) -> Self {
        self.override_redirect = true;
        self
    }
    fn transient_for(mut self, parent: Window) -> Self {
        self.transient_for = Some(parent);
        self
    }
    fn wm_type(mut self, t: WmType) -> Self {
        self.wm_type = t;
        self
    }
}

/// An XDND drag this process is the SOURCE of.
///
/// The protocol is entirely client-to-client: the source picks the window under the
/// pointer itself, checks its `XdndAware`, and sends it root coordinates. Nothing in it
/// consults the window manager — which is the point of testing it, because y5's answer to
/// "where is this window" and X's answer are different by construction.
struct Drag {
    source: Window,
    /// The window currently being told about the drag, and its XDND version.
    target: Option<(Window, u32)>,
}

struct Win {
    id: u32,
    window: Window,
    label: String,
    override_redirect: bool,
    /// Created on a 32-bit visual, so its contents carry alpha.
    argb: bool,
    /// The type this window was created with or last set to. Reported by `info` so a
    /// classification surprise can be read off one line.
    wm_type: WmType,
    /// The parent it names, if any — the other half of the popup decision.
    transient_for: Option<Window>,
    /// The last `ConfigureNotify` the server sent us — i.e. where the COMPOSITOR put
    /// this window, which for a managed window is the interesting number.
    last_configure: (i16, i16, u16, u16),
    /// What we asked for, so a divergence is visible in `info`.
    wanted: (u16, u16),
    colour: u32,
}

struct Subject {
    conn: RustConnection,
    screen: usize,
    root: Window,
    atoms: Atoms,
    gc: Gcontext,
    text_gc: Gcontext,
    wins: BTreeMap<u32, Win>,
    next_id: u32,
    /// The text this process currently offers on `CLIPBOARD`, if it owns it.
    clipboard: Option<String>,
    /// Last time a motion line was printed, for the throttle in `on_event`.
    last_motion_log: Option<std::time::Instant>,
    /// `_NET_WM_ICON` to write on the next window created, before its map request —
    /// which is when applications really set it, and when the compositor first reads it.
    icon_next: Option<IconSpec>,
    /// The drag in progress, if any — see [`Drag`].
    drag: Option<Drag>,
    /// Graphics contexts for depth-32 drawables, made from the first ARGB window.
    ///
    /// A GC is bound to a DEPTH, not to a window, so one pair serves every ARGB window —
    /// but the depth-24 pair made against the root cannot draw into them at all (a
    /// `BadMatch`), which is why these exist separately rather than as a switch.
    argb_gcs: Option<(Gcontext, Gcontext)>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::intern(&conn)?;

    // A core font, so the harness has no font dependency at all. `fixed` is present
    // on every X server worth testing against; if it is missing we simply draw no
    // labels rather than refusing to start.
    let font = conn.generate_id()?;
    let have_font = conn.open_font(font, b"fixed").is_ok();

    let gc = conn.generate_id()?;
    conn.create_gc(gc, root, &CreateGCAux::new())?;
    let text_gc = conn.generate_id()?;
    let mut text_aux = CreateGCAux::new().foreground(0x101010);
    if have_font {
        text_aux = text_aux.font(font);
    }
    conn.create_gc(text_gc, root, &text_aux)?;
    conn.flush()?;

    eprintln!(
        "[subject] connected display={} pid={} root={root:#x}",
        std::env::var("DISPLAY").unwrap_or_else(|_| "<unset>".into()),
        std::process::id()
    );
    eprintln!("[subject] _NET_WM_PID will be set to {} on every window", std::process::id());

    let mut me = Subject {
        conn,
        screen,
        root,
        atoms,
        gc,
        text_gc,
        wins: BTreeMap::new(),
        next_id: 1,
        clipboard: None,
        last_motion_log: None,
        icon_next: None,
        drag: None,
        argb_gcs: None,
    };

    // stdin on its own thread: the X connection has to stay pumped while we wait for
    // a line, or the compositor's configures pile up unanswered and every geometry
    // the harness reports is stale.
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    loop {
        while let Some(event) = me.conn.poll_for_event()? {
            if me.on_event(event)? {
                return Ok(());
            }
        }
        match rx.try_recv() {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                match Command::parse(&line) {
                    Some(Command::Quit) => {
                        eprintln!("[subject] quit");
                        return Ok(());
                    }
                    Some(cmd) => {
                        if let Err(err) = me.run(cmd) {
                            eprintln!("[subject] error: {err}");
                        }
                    }
                    None => eprintln!("[subject] unknown command: {line:?}"),
                }
                me.conn.flush()?;
            }
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {
                // The drag is driven from HERE rather than from motion events: the source
                // owns the pointer during an XDND drag and the target's own motion events
                // stop, so polling `query_pointer` is what a real toolkit does too.
                if let Err(err) = me.pump_drag() {
                    eprintln!("[subject] dnd pump error: {err}");
                }
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
        }
    }
}

impl Subject {
    fn win(&self, id: u32) -> Result<&Win, String> {
        self.wins.get(&id).ok_or_else(|| format!("no window {id}"))
    }

    /// Create a window. `override_redirect` is what makes it a menu rather than a
    /// toplevel, and is the one bit the whole override-redirect question turns on.
    fn create(&mut self, spec: Spec) -> Result<u32, Box<dyn std::error::Error>> {
        let Spec { x, y, w, h, override_redirect, transient_for, wm_type, argb } = spec;
        let id = self.next_id;
        self.next_id += 1;
        let window = self.conn.generate_id()?;
        let colour = if override_redirect {
            OVERRIDE[(id as usize) % OVERRIDE.len()]
        } else {
            MANAGED[(id as usize) % MANAGED.len()]
        };
        let screen = &self.conn.setup().roots[self.screen];
        // Depth 32 needs its own visual AND its own colormap, and the border pixel must
        // be given explicitly: a window whose depth differs from its parent's inherits
        // neither, and leaving either out is a `BadMatch` at CreateWindow rather than a
        // wrong-looking window. Background fully TRANSPARENT (0x00000000), so the window
        // starts as a hole and every visible pixel is one this program drew.
        let argb_visual = argb.then(|| argb_visual(screen)).flatten();
        if argb && argb_visual.is_none() {
            eprintln!("[subject] no 32-bit visual on this screen; creating an opaque window instead");
        }
        let (depth, visual, colormap) = match argb_visual {
            Some(visual) => {
                let cid = self.conn.generate_id()?;
                self.conn.create_colormap(ColormapAlloc::NONE, cid, self.root, visual)?;
                (32u8, visual, Some(cid))
            }
            None => (x11rb::COPY_DEPTH_FROM_PARENT, screen.root_visual, None),
        };
        let argb = colormap.is_some();
        let mut aux = CreateWindowAux::new()
            .background_pixel(if argb { 0x0000_0000 } else { colour })
            .override_redirect(u32::from(override_redirect))
            // ENTER/LEAVE and motion as well as clicks. A click says input arrived;
            // a CROSSING says which window the X server's own hit test picked, which
            // is the thing y5's stacking is trying to steer — an overlapping menu
            // that never reports an enter is the failure, and it happens without any
            // click being lost to notice it by.
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::KEY_PRESS
                    | EventMask::ENTER_WINDOW
                    | EventMask::LEAVE_WINDOW
                    | EventMask::POINTER_MOTION
                    | EventMask::PROPERTY_CHANGE,
            );
        if let Some(cid) = colormap {
            aux = aux.colormap(cid).border_pixel(0);
        }
        self.conn.create_window(
            depth,
            window,
            self.root,
            x,
            y,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )?;
        if argb && self.argb_gcs.is_none() {
            // Made against this window because a GC is bound to a depth, and there was no
            // depth-32 drawable to make one from until now. Reused by every later ARGB
            // window.
            let gc = self.conn.generate_id()?;
            self.conn.create_gc(gc, window, &CreateGCAux::new())?;
            let text = self.conn.generate_id()?;
            self.conn.create_gc(text, window, &CreateGCAux::new().foreground(0xff10_1010))?;
            self.argb_gcs = Some((gc, text));
        }

        // Identity, always. `_NET_WM_PID` is what a native XWM reads to answer "which
        // process owns this window" — the surface credentials name the one Xwayland
        // client for every X11 window, so this property is the only per-window truth.
        self.conn.change_property32(
            PropMode::REPLACE,
            window,
            self.atoms.net_wm_pid,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        let label = format!("win {id}");
        self.set_title(window, &label)?;
        self.set_class(window, "y5stress")?;
        // Offer WM_DELETE_WINDOW by default: the polite close is the common case, and
        // `close-mode nodelete` is how the other path is asked for.
        self.conn.change_property32(
            PropMode::REPLACE,
            window,
            self.atoms.wm_protocols,
            AtomEnum::ATOM,
            &[self.atoms.wm_delete_window],
        )?;
        if let Some(parent) = transient_for {
            self.conn.change_property32(
                PropMode::REPLACE,
                window,
                AtomEnum::WM_TRANSIENT_FOR,
                AtomEnum::WINDOW,
                &[parent],
            )?;
        }
        // BEFORE the map request, which is when the compositor first classifies the
        // window. Setting it afterwards is a different test (`window-type`), and one
        // worth running separately — see the `type-flip` scenario.
        self.apply_window_type(window, wm_type)?;
        // The icon too, and for the same reason: applications set `_NET_WM_ICON` before
        // mapping, and the compositor resolves it once per window.
        if let Some(spec) = self.icon_next.take() {
            self.apply_icon(window, spec, colour)?;
            eprintln!(
                "[subject]   icon={} written before map, bordered {colour:#08x} (this window's colour)",
                spec.token()
            );
        }

        self.conn.map_window(window)?;
        self.conn.flush()?;
        self.wins.insert(
            id,
            Win {
                id,
                window,
                label,
                override_redirect,
                argb,
                wm_type,
                transient_for,
                last_configure: (x, y, w, h),
                wanted: (w, h),
                colour,
            },
        );
        eprintln!(
            "[subject] created {id} window={window:#x} {w}x{h}+{x}+{y} \
             override_redirect={override_redirect} argb={argb} type={} transient_for={}",
            wm_type.token(),
            match transient_for {
                Some(p) => format!("{p:#x}"),
                None => "none".into(),
            }
        );
        if override_redirect && transient_for.is_none() {
            eprintln!(
                "[subject]   note: override-redirect with NO parent is a WINDOW to y5, \
                 not a popup — a popup needs a resolvable WM_TRANSIENT_FOR"
            );
        }
        Ok(id)
    }

    /// Write (or delete) `_NET_WM_WINDOW_TYPE`.
    fn apply_window_type(&self, window: Window, t: WmType) -> Result<(), Box<dyn std::error::Error>> {
        match self.atoms.window_types.iter().find(|(k, _)| *k == t) {
            Some((_, atom)) => {
                self.conn.change_property32(
                    PropMode::REPLACE,
                    window,
                    self.atoms.net_wm_window_type,
                    AtomEnum::ATOM,
                    &[*atom],
                )?;
            }
            // `WmType::Unset` — no entry, so delete the property and let the
            // compositor fall back to its no-type heuristic.
            None => {
                self.conn.delete_property(window, self.atoms.net_wm_window_type)?;
            }
        }
        Ok(())
    }

    /// Write (or delete) `_NET_WM_ICON`.
    fn apply_icon(
        &self,
        window: Window,
        spec: IconSpec,
        colour: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let words = icon_words(spec, colour);
        if words.is_empty() {
            self.conn.delete_property(window, self.atoms.net_wm_icon)?;
            return Ok(());
        }
        self.conn.change_property32(
            PropMode::REPLACE,
            window,
            self.atoms.net_wm_icon,
            AtomEnum::CARDINAL,
            &words,
        )?;
        Ok(())
    }

    /// Add or remove one `_NET_WM_STATE` atom via a client message, which is what a
    /// mapped window must do (writing the property directly only works before map).
    fn wm_state(&self, window: Window, atom: Atom, on: bool) -> Result<(), Box<dyn std::error::Error>> {
        let data = [u32::from(on), atom, 0, 1, 0];
        let event = ClientMessageEvent::new(32, window, self.atoms.net_wm_state, data);
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )?;
        Ok(())
    }

    fn set_title(&self, window: Window, title: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Both spellings: WM_NAME is what ICCCM defines and _NET_WM_NAME is what EWMH
        // added for UTF-8. smithay reads the EWMH one first and falls back.
        self.conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        )?;
        self.conn.change_property8(
            PropMode::REPLACE,
            window,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            title.as_bytes(),
        )?;
        Ok(())
    }

    /// `WM_CLASS` is two NUL-terminated strings: instance then class. The compositor
    /// takes the CLASS half as `app_id`, so both are written and only the second one
    /// is expected to show up in a dock.
    fn set_class(&self, window: Window, class: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(class.to_lowercase().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(class.as_bytes());
        bytes.push(0);
        self.conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            &bytes,
        )?;
        Ok(())
    }

    /// `WM_HINTS` + `WM_PROTOCOLS` for one of the four ICCCM input models.
    ///
    /// `WM_HINTS` is nine CARD32s; bit 0 of `flags` is `InputHint` and field 1 is the
    /// input flag itself. Everything else stays zero — this harness has no icon, no
    /// initial state and no window group to declare.
    fn set_input_model(&self, w: &Win, model: InputModel) -> Result<(), Box<dyn std::error::Error>> {
        let input = matches!(model, InputModel::Passive | InputModel::LocallyActive);
        let hints: [u32; 9] = [1, u32::from(input), 0, 0, 0, 0, 0, 0, 0];
        self.conn.change_property32(
            PropMode::REPLACE,
            w.window,
            AtomEnum::WM_HINTS,
            AtomEnum::WM_HINTS,
            &hints,
        )?;
        let take_focus = matches!(model, InputModel::LocallyActive | InputModel::GloballyActive);
        let mut protocols = vec![self.atoms.wm_delete_window];
        if take_focus {
            protocols.push(self.atoms.wm_take_focus);
        }
        self.conn.change_property32(
            PropMode::REPLACE,
            w.window,
            self.atoms.wm_protocols,
            AtomEnum::ATOM,
            &protocols,
        )?;
        eprintln!(
            "[subject] {} input-model={} (input={input} take_focus={take_focus})",
            w.id,
            model.token()
        );
        Ok(())
    }

    fn run(&mut self, cmd: Command) -> Result<(), Box<dyn std::error::Error>> {
        match cmd {
            Command::Map(w, h) => {
                // Stagger so consecutive windows are not exactly on top of each other
                // when the compositor honours the requested position (it should not).
                let n = self.next_id as i16;
                self.create(Spec::managed(40 * n, 40 * n, w, h))?;
            }
            Command::MapArgb(w, h) => {
                let n = self.next_id as i16;
                self.create(Spec::managed(40 * n, 40 * n, w, h).argb())?;
            }
            Command::Opacity(id, value) => {
                let window = self.win(id)?.window;
                // The property is a CARD32 over the full 32-bit range, not 0-255: a
                // compositing manager reads `0xffffffff` as opaque, so the byte is
                // scaled rather than written straight (0x80 becomes 0x80808080).
                let scaled = u32::from(value) * 0x0101_0101;
                self.conn.change_property32(
                    PropMode::REPLACE,
                    window,
                    self.atoms.net_wm_window_opacity,
                    AtomEnum::CARDINAL,
                    &[scaled],
                )?;
                eprintln!(
                    "[subject] {id} _NET_WM_WINDOW_OPACITY={scaled:#010x} ({value}/255) — a \
                     compositing-manager convention, not a protocol; y5 may ignore it"
                );
            }
            Command::MapTransient(parent, w, h) => {
                let p = self.win(parent)?.window;
                let n = self.next_id as i16;
                self.create(Spec::managed(60 * n, 60 * n, w, h).transient_for(p))?;
            }
            Command::Unmap(id) => {
                let w = self.win(id)?.window;
                self.conn.unmap_window(w)?;
                self.conn.flush()?;
                eprintln!(
                    "[subject] {id} unmapped (WITHDRAWN) — the X window still exists; \
                     `remap {id}` brings it back, `destroy {id}` ends it"
                );
            }
            Command::Remap(id) => {
                let w = self.win(id)?.window;
                self.conn.map_window(w)?;
                self.conn.flush()?;
                eprintln!("[subject] {id} mapped again");
            }
            Command::Destroy(id) => {
                let w = self.win(id)?.window;
                self.conn.destroy_window(w)?;
                self.wins.remove(&id);
                eprintln!("[subject] destroyed {id}");
            }
            Command::Title(id, t) => {
                let w = self.win(id)?;
                let (window, wid) = (w.window, w.id);
                self.set_title(window, &t)?;
                if let Some(w) = self.wins.get_mut(&wid) {
                    w.label = t.clone();
                }
                self.redraw(wid)?;
                eprintln!("[subject] {id} title={t:?}");
            }
            Command::Class(id, c) => {
                let window = self.win(id)?.window;
                self.set_class(window, &c)?;
                eprintln!("[subject] {id} class={c:?}");
            }
            Command::Pid(id, pid) => {
                let window = self.win(id)?.window;
                if pid == 0 {
                    self.conn.delete_property(window, self.atoms.net_wm_pid)?;
                    eprintln!("[subject] {id} _NET_WM_PID cleared");
                } else {
                    self.conn.change_property32(
                        PropMode::REPLACE,
                        window,
                        self.atoms.net_wm_pid,
                        AtomEnum::CARDINAL,
                        &[pid],
                    )?;
                    eprintln!("[subject] {id} _NET_WM_PID={pid}");
                }
            }
            Command::Fullscreen(id, on) => {
                let window = self.win(id)?.window;
                // EWMH: ask the window manager rather than setting the property, which
                // is what a real client does and what reaches `fullscreen_request`.
                let data = [
                    u32::from(on), // 1 = _NET_WM_STATE_ADD, 0 = _REMOVE
                    self.atoms.net_wm_state_fullscreen,
                    0,
                    1, // source indication: normal application
                    0,
                ];
                let event = ClientMessageEvent::new(32, window, self.atoms.net_wm_state, data);
                self.conn.send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                    event,
                )?;
                eprintln!("[subject] {id} fullscreen={on}");
            }
            Command::InputModel(id, model) => {
                let w = self.win(id)?;
                let w = Win {
                    id: w.id,
                    window: w.window,
                    label: w.label.clone(),
                    override_redirect: w.override_redirect,
                    argb: w.argb,
                    wm_type: w.wm_type,
                    transient_for: w.transient_for,
                    last_configure: w.last_configure,
                    wanted: w.wanted,
                    colour: w.colour,
                };
                self.set_input_model(&w, model)?;
            }
            Command::CloseMode(id, mode) => {
                let window = self.win(id)?.window;
                let protocols: Vec<Atom> = match mode {
                    CloseMode::Delete => vec![self.atoms.wm_delete_window],
                    CloseMode::NoDelete => vec![],
                };
                self.conn.change_property32(
                    PropMode::REPLACE,
                    window,
                    self.atoms.wm_protocols,
                    AtomEnum::ATOM,
                    &protocols,
                )?;
                eprintln!(
                    "[subject] {id} close-mode={} (a no-delete window is DESTROYED by a WM, \
                     and the compositor must not also signal the pid)",
                    match mode {
                        CloseMode::Delete => "delete",
                        CloseMode::NoDelete => "nodelete",
                    }
                );
            }
            Command::Or(parent, dx, dy, w, h) => {
                let p = self.win(parent)?;
                let (px, py) = (p.last_configure.0, p.last_configure.1);
                // Absolute root coordinates, which is the ONLY way X lets a client
                // place an override-redirect window — there is no parent-relative
                // form. It works out because the compositor configures managed X11
                // windows at their y5-world location, so the two agree by
                // construction.
                self.create(Spec::managed(px + dx, py + dy, w, h).override_redirect())?;
            }
            Command::OrChain(parent, dx, dy) => {
                let p = self.win(parent)?;
                let (px, py, pw, ph) = p.last_configure;
                let _ = (pw, ph);
                self.create(Spec::managed(px + dx, py + dy, 180, 140).override_redirect())?;
            }
            Command::OrReanchor(id, dx, dy) => {
                let w = self.win(id)?;
                if !w.override_redirect {
                    eprintln!("[subject] {id} is not override-redirect; re-anchor is meaningless");
                    return Ok(());
                }
                let (x, y) = (w.last_configure.0 + dx, w.last_configure.1 + dy);
                let window = w.window;
                // An override-redirect window moves itself with a plain
                // ConfigureWindow — no window manager is supposed to be involved.
                self.conn.configure_window(
                    window,
                    &ConfigureWindowAux::new().x(i32::from(x)).y(i32::from(y)),
                )?;
                eprintln!("[subject] {id} re-anchored to +{x}+{y} (client-side)");
            }
            Command::MapChild(parent, dx, dy, w, h, or, t) => {
                let p = self.win(parent)?;
                let (px, py) = (p.last_configure.0, p.last_configure.1);
                let pw = p.window;
                // Absolute root coordinates: X has no parent-relative form for a
                // top-level child, so the harness does the addition itself from where
                // the SERVER last said the parent is.
                let mut spec = Spec::managed(px + dx, py + dy, w, h)
                    .transient_for(pw)
                    .wm_type(t);
                if or {
                    spec = spec.override_redirect();
                }
                self.create(spec)?;
            }
            Command::WindowType(id, t) => {
                let window = self.win(id)?.window;
                self.apply_window_type(window, t)?;
                if let Some(w) = self.wins.get_mut(&id) {
                    w.wm_type = t;
                }
                eprintln!(
                    "[subject] {id} _NET_WM_WINDOW_TYPE={} (set AFTER map — the \
                     compositor classified this window already)",
                    t.token()
                );
            }
            Command::Icon(id, spec) => {
                let w = self.win(id)?;
                let (window, colour) = (w.window, w.colour);
                self.apply_icon(window, spec, colour)?;
                eprintln!(
                    "[subject] {id} _NET_WM_ICON={} set AFTER map — the compositor may have \
                     resolved this window's icon already",
                    spec.token()
                );
            }
            Command::IconNext(spec) => {
                self.icon_next = Some(spec);
                eprintln!("[subject] next window will carry _NET_WM_ICON={}", spec.token());
            }
            Command::Modal(id, on) => {
                let window = self.win(id)?.window;
                self.wm_state(window, self.atoms.net_wm_state_modal, on)?;
                eprintln!("[subject] {id} _NET_WM_STATE_MODAL={on}");
            }
            Command::SkipTaskbar(id, on) => {
                let window = self.win(id)?.window;
                self.wm_state(window, self.atoms.net_wm_state_skip_taskbar, on)?;
                eprintln!(
                    "[subject] {id} _NET_WM_STATE_SKIP_TASKBAR={on} (only consulted \
                     when no window type is declared)"
                );
            }
            Command::TransientFor(id, parent) => {
                let window = self.win(id)?.window;
                let parent_window = if parent == 0 {
                    None
                } else {
                    Some(self.win(parent)?.window)
                };
                match parent_window {
                    Some(p) => self.conn.change_property32(
                        PropMode::REPLACE,
                        window,
                        AtomEnum::WM_TRANSIENT_FOR,
                        AtomEnum::WINDOW,
                        &[p],
                    )?,
                    None => self
                        .conn
                        .delete_property(window, Atom::from(AtomEnum::WM_TRANSIENT_FOR))?,
                };
                if let Some(w) = self.wins.get_mut(&id) {
                    w.transient_for = parent_window;
                }
                eprintln!(
                    "[subject] {id} WM_TRANSIENT_FOR={}",
                    match parent_window {
                        Some(p) => format!("{p:#x}"),
                        None => "cleared".into(),
                    }
                );
            }
            Command::SelfMove(id, x, y) => self.self_configure(id, Some((x, y)), None)?,
            Command::SelfResize(id, w, h) => self.self_configure(id, None, Some((w, h)))?,
            Command::SelfMoveResize(id, x, y, w, h) => {
                self.self_configure(id, Some((x, y)), Some((w, h)))?
            }
            Command::ResizeRequest(id, w, h) => {
                let win = self.win(id)?;
                let window = win.window;
                let wid = win.id;
                self.conn.configure_window(
                    window,
                    &ConfigureWindowAux::new().width(u32::from(w)).height(u32::from(h)),
                )?;
                if let Some(win) = self.wins.get_mut(&wid) {
                    win.wanted = (w, h);
                }
                eprintln!("[subject] {id} asked for {w}x{h} — the compositor SHOULD honour this");
            }
            Command::MoveRequest(id, x, y) => {
                let window = self.win(id)?.window;
                self.conn.configure_window(
                    window,
                    &ConfigureWindowAux::new().x(i32::from(x)).y(i32::from(y)),
                )?;
                eprintln!(
                    "[subject] {id} asked to move to +{x}+{y} — the compositor SHOULD REFUSE this"
                );
            }
            Command::Raise(id) | Command::Lower(id) => {
                let above = matches!(cmd, Command::Raise(_));
                let window = self.win(id)?.window;
                let mode = if above { StackMode::ABOVE } else { StackMode::BELOW };
                self.conn
                    .configure_window(window, &ConfigureWindowAux::new().stack_mode(mode))?;
                eprintln!("[subject] {id} asked to {}", if above { "raise" } else { "lower" });
            }
            Command::DndTarget(id) => {
                let window = self.win(id)?.window;
                // Version 5, which is what every real toolkit speaks.
                self.conn.change_property32(
                    PropMode::REPLACE,
                    window,
                    self.atoms.xdnd_aware,
                    AtomEnum::ATOM,
                    &[5],
                )?;
                eprintln!("[subject] {id} XdndAware=5 — now a drop target");
            }
            Command::DndSource(id) => {
                let window = self.win(id)?.window;
                self.conn.set_selection_owner(
                    window,
                    self.atoms.xdnd_selection,
                    x11rb::CURRENT_TIME,
                )?;
                self.drag = Some(Drag { source: window, target: None });
                eprintln!(
                    "[subject] {id} drag started — move the pointer over a dnd-target window; \
                     `dnd-drop` to finish, `dnd-cancel` to abandon"
                );
            }
            Command::DndDrop => {
                let Some(drag) = self.drag.take() else {
                    eprintln!("[subject] no drag in progress");
                    return Ok(());
                };
                if let Some((target, _)) = drag.target {
                    let data = [drag.source, 0, x11rb::CURRENT_TIME, 0, 0];
                    self.send_xdnd(target, self.atoms.xdnd_drop, data)?;
                    eprintln!("[subject] dnd source: XdndDrop -> {target:#010x}");
                } else {
                    eprintln!("[subject] dnd source: dropped over nothing");
                }
            }
            Command::DndCancel => {
                let Some(drag) = self.drag.take() else {
                    eprintln!("[subject] no drag in progress");
                    return Ok(());
                };
                if let Some((target, _)) = drag.target {
                    let data = [drag.source, 0, 0, 0, 0];
                    self.send_xdnd(target, self.atoms.xdnd_leave, data)?;
                }
                eprintln!("[subject] dnd source: cancelled");
            }
            Command::Copy(text) => {
                let owner = self.wins.values().next().map(|w| w.window);
                let Some(owner) = owner else {
                    eprintln!("[subject] copy needs at least one window to own the selection");
                    return Ok(());
                };
                self.conn.set_selection_owner(owner, self.atoms.clipboard, x11rb::CURRENT_TIME)?;
                self.clipboard = Some(text.clone());
                eprintln!("[subject] took CLIPBOARD with {text:?} — now paste in a wayland app");
            }
            Command::Paste => {
                let requestor = self.wins.values().next().map(|w| w.window);
                let Some(requestor) = requestor else {
                    eprintln!("[subject] paste needs at least one window to receive on");
                    return Ok(());
                };
                self.conn.convert_selection(
                    requestor,
                    self.atoms.clipboard,
                    self.atoms.utf8_string,
                    self.atoms.stress_sel,
                    x11rb::CURRENT_TIME,
                )?;
                eprintln!("[subject] asked for CLIPBOARD as UTF8_STRING…");
            }
            Command::StormResize(id, n) => {
                let win = self.win(id)?;
                let (window, base) = (win.window, win.wanted);
                for i in 0..n {
                    let d = (i % 40) as u16;
                    self.conn.configure_window(
                        window,
                        &ConfigureWindowAux::new()
                            .width(u32::from(base.0.saturating_add(d)))
                            .height(u32::from(base.1.saturating_add(d))),
                    )?;
                }
                self.conn.flush()?;
                eprintln!("[subject] {id} sent {n} resize requests back-to-back");
            }
            Command::StormReanchor(id, n) => {
                let win = self.win(id)?;
                let (window, base) = (win.window, win.last_configure);
                for i in 0..n {
                    let d = (i % 40) as i16;
                    self.conn.configure_window(
                        window,
                        &ConfigureWindowAux::new()
                            .x(i32::from(base.0 + d))
                            .y(i32::from(base.1 + d)),
                    )?;
                }
                self.conn.flush()?;
                eprintln!("[subject] {id} sent {n} re-anchors back-to-back");
            }
            Command::Info => self.info()?,
            Command::Stack => self.stack()?,
            Command::Pointer => self.pointer()?,
            Command::Quit => {}
        }
        Ok(())
    }

    /// One `ConfigureWindow` carrying whatever was asked for.
    ///
    /// The same request means two different things and the log says which is expected:
    /// for an override-redirect window the server just does it (no WM is consulted),
    /// while for a managed window it is redirected to the compositor as a
    /// `ConfigureRequest` — which y5 honours for SIZE and refuses for POSITION. Watch
    /// the `configured by the compositor` line that follows, or its absence.
    fn self_configure(
        &mut self,
        id: u32,
        pos: Option<(i16, i16)>,
        size: Option<(u16, u16)>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let w = self.win(id)?;
        let (window, or, wid) = (w.window, w.override_redirect, w.id);
        let mut aux = ConfigureWindowAux::new();
        if let Some((x, y)) = pos {
            aux = aux.x(i32::from(x)).y(i32::from(y));
        }
        if let Some((width, height)) = size {
            aux = aux.width(u32::from(width)).height(u32::from(height));
        }
        self.conn.configure_window(window, &aux)?;
        self.conn.flush()?;
        if let (Some(sz), Some(w)) = (size, self.wins.get_mut(&wid)) {
            w.wanted = sz;
        }
        let what = match (pos, size) {
            (Some(p), Some(s)) => format!("move to +{}+{} and resize to {}x{}", p.0, p.1, s.0, s.1),
            (Some(p), None) => format!("move to +{}+{}", p.0, p.1),
            (None, Some(s)) => format!("resize to {}x{}", s.0, s.1),
            (None, None) => "nothing".into(),
        };
        let expect = if or {
            "override-redirect: the SERVER applies this directly, no WM involved"
        } else {
            "managed: this is a ConfigureRequest — size SHOULD be honoured, position REFUSED"
        };
        eprintln!("[subject] {id} self-configure: {what} — {expect}");
        Ok(())
    }

    /// Paint the ARGB window: vertical alpha bands, clear on the left to opaque on the
    /// right, inside a fully opaque frame.
    ///
    /// The frame is what keeps the window findable. A fully transparent left edge looks
    /// the same as "nothing was drawn" and as "the compositor culled it", and those are
    /// the three outcomes this window exists to tell apart.
    ///
    /// Colours are PREMULTIPLIED, which is what an X ARGB visual means by alpha: every
    /// channel is scaled by the alpha rather than left at full strength. A compositor
    /// treating them as straight washes the ramp out toward the transparent end instead
    /// of fading it, so that mistake is visible rather than merely wrong.
    fn paint_alpha_ramp(
        &self,
        w: &Win,
        gc: Gcontext,
        width: u16,
        height: u16,
    ) -> Result<(), Box<dyn std::error::Error>> {
        const BANDS: u16 = 24;
        let band = (width / BANDS).max(1);
        for i in 0..BANDS {
            let x = i * band;
            if x >= width {
                break;
            }
            let alpha = (u32::from(i) * 255) / u32::from(BANDS - 1);
            let premul = |shift: u32| (((w.colour >> shift) & 0xff) * alpha / 255) << shift;
            let pixel = (alpha << 24) | premul(16) | premul(8) | premul(0);
            self.conn.change_gc(gc, &ChangeGCAux::new().foreground(pixel))?;
            self.conn.poly_fill_rectangle(
                w.window,
                gc,
                &[Rectangle { x: x as i16, y: 0, width: band.min(width - x), height }],
            )?;
        }
        // Opaque frame last, so it survives whatever the bands did. Saturating, because a
        // window can be configured smaller than the frame is thick and an underflow here
        // would take the subject down over a cosmetic border.
        self.conn.change_gc(gc, &ChangeGCAux::new().foreground(0xff00_0000 | w.colour))?;
        let t = 3u16.min(width).min(height);
        for rect in [
            Rectangle { x: 0, y: 0, width, height: t },
            Rectangle { x: 0, y: height.saturating_sub(t) as i16, width, height: t },
            Rectangle { x: 0, y: 0, width: t, height },
            Rectangle { x: width.saturating_sub(t) as i16, y: 0, width: t, height },
        ] {
            self.conn.poly_fill_rectangle(w.window, gc, &[rect])?;
        }
        Ok(())
    }

    /// One XDND client message.
    fn send_xdnd(
        &self,
        to: Window,
        type_: Atom,
        data: [u32; 5],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let event = ClientMessageEvent::new(32, to, type_, data);
        self.conn.send_event(false, to, EventMask::NO_EVENT, event)?;
        self.conn.flush()?;
        Ok(())
    }

    /// The first `XdndAware` window at or under `w`, with its declared version.
    ///
    /// A search rather than a lookup because `query_pointer` returns the immediate child
    /// of the ROOT, which under a reparenting window manager is the FRAME smithay created,
    /// not the client's window — the aware property lives further down.
    fn first_aware(&self, w: Window, depth: u8) -> Option<(Window, u32)> {
        if let Ok(reply) = self
            .conn
            .get_property(false, w, self.atoms.xdnd_aware, AtomEnum::ATOM, 0, 1)
            .and_then(|c| Ok(c.reply()))
        {
            if let Ok(reply) = reply {
                if let Some(mut v) = reply.value32() {
                    if let Some(version) = v.next() {
                        return Some((w, version));
                    }
                }
            }
        }
        if depth == 0 {
            return None;
        }
        let tree = self.conn.query_tree(w).ok()?.reply().ok()?;
        tree.children
            .into_iter()
            .rev()
            .find_map(|c| self.first_aware(c, depth - 1))
    }

    /// Drive the drag one step: where does the SOURCE think the pointer is, and which
    /// window does it pick?
    ///
    /// This is the measurement. Everything here is the client's own arithmetic — the
    /// pointer position comes from `query_pointer` in ROOT coordinates and goes into
    /// `XdndPosition` unmodified, and the window is chosen by walking the X tree. y5 is
    /// not consulted at any point, and cannot be: the protocol has no room for it.
    fn pump_drag(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(drag) = self.drag.as_ref() else { return Ok(()) };
        let (source, previous) = (drag.source, drag.target);
        let p = self.conn.query_pointer(self.root)?.reply()?;
        let found = if p.child == x11rb::NONE {
            None
        } else {
            self.first_aware(p.child, 3)
        };
        if found.map(|(w, _)| w) != previous.map(|(w, _)| w) {
            if let Some((old, _)) = previous {
                let data = [source, 0, 0, 0, 0];
                self.send_xdnd(old, self.atoms.xdnd_leave, data)?;
                eprintln!("[subject] dnd source: XdndLeave -> {old:#010x}");
            }
            if let Some((new, version)) = found {
                let data = [source, version << 24, self.atoms.text_plain, 0, 0];
                self.send_xdnd(new, self.atoms.xdnd_enter, data)?;
                eprintln!(
                    "[subject] dnd source: XdndEnter -> {new:#010x} (XdndAware v{version})"
                );
            }
            if let Some(drag) = self.drag.as_mut() {
                drag.target = found;
            }
        }
        if let Some((target, _)) = found {
            let packed = ((p.root_x as u32) << 16) | (p.root_y as u32 & 0xffff);
            let data = [source, 0, packed, x11rb::CURRENT_TIME, self.atoms.xdnd_action_copy];
            self.send_xdnd(target, self.atoms.xdnd_position, data)?;
            eprintln!(
                "[subject] dnd source: XdndPosition root=({},{}) child={:#010x} target={target:#010x}",
                p.root_x, p.root_y, p.child
            );
        }
        Ok(())
    }

    /// Where the SERVER says the pointer is, and whose window it landed in.
    ///
    /// The direct read on y5's stacking mechanism. An ungrabbed pointer event reaches
    /// whichever client the X server's own hit test picks, and the X layout owes
    /// nothing to the canvas — so when input goes to the wrong client, this is the
    /// line that says so, and `stack` is where the reason will be.
    fn pointer(&self) -> Result<(), Box<dyn std::error::Error>> {
        let p = self.conn.query_pointer(self.root)?.reply()?;
        let named = self
            .wins
            .values()
            .find(|w| w.window == p.child)
            .map(|w| format!("ours: {} {:?}", w.id, w.label))
            .unwrap_or_else(|| "not one of ours".into());
        eprintln!(
            "[subject] pointer at +{}+{} root, child={:#010x} ({named}) same_screen={}",
            p.root_x, p.root_y, p.child, p.same_screen
        );
        Ok(())
    }

    /// What the SERVER thinks, not what we asked for — the divergence is the point.
    fn info(&self) -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("[subject] --- windows (pid {}) ---", std::process::id());
        for w in self.wins.values() {
            let geom = self.conn.get_geometry(w.window)?.reply()?;
            // `translate_coordinates` against the root is how a client learns where it
            // actually IS: `get_geometry` reports x/y relative to the parent, which
            // under a reparenting WM is not the root at all.
            let abs = self
                .conn
                .translate_coordinates(w.window, self.root, 0, 0)?
                .reply()?;
            eprintln!(
                "[subject]  {:>2} x11={:#010x} or={:<5} type={:<14} parent={:<10} \
                 asked={}x{} server={}x{} at +{}+{} label={:?}",
                w.id,
                w.window,
                w.override_redirect,
                w.wm_type.token(),
                match w.transient_for {
                    Some(p) => format!("{p:#x}"),
                    None => "none".into(),
                },
                w.wanted.0,
                w.wanted.1,
                geom.width,
                geom.height,
                abs.dst_x,
                abs.dst_y,
                w.label,
            );
        }
        Ok(())
    }

    /// The root's children, bottom-to-top, with ours marked. This is the compositor's
    /// stacking mirror as the X SERVER sees it — the thing a client consults when it
    /// decides where to put its own menu.
    fn stack(&self) -> Result<(), Box<dyn std::error::Error>> {
        let tree = self.conn.query_tree(self.root)?.reply()?;
        eprintln!("[subject] --- root stack, bottom to top ---");
        for child in tree.children {
            if let Some(w) = self.wins.values().find(|w| w.window == child) {
                eprintln!("[subject]  {:#010x}  <== ours: {} {:?}", child, w.id, w.label);
            } else {
                eprintln!("[subject]  {child:#010x}");
            }
        }
        Ok(())
    }

    fn redraw(&self, id: u32) -> Result<(), Box<dyn std::error::Error>> {
        let Some(w) = self.wins.get(&id) else { return Ok(()) };
        let geom = self.conn.get_geometry(w.window)?.reply()?;
        // A GC is bound to a DEPTH, so the depth-24 pair made against the root cannot
        // draw into a depth-32 window at all — that is a `BadMatch`, not a wrong colour.
        let (gc, text_gc) = match (w.argb, self.argb_gcs) {
            (true, Some(pair)) => pair,
            _ => (self.gc, self.text_gc),
        };
        if w.argb {
            self.paint_alpha_ramp(w, gc, geom.width, geom.height)?;
        } else {
            self.conn.change_gc(gc, &ChangeGCAux::new().foreground(w.colour))?;
            self.conn.poly_fill_rectangle(
                w.window,
                gc,
                &[Rectangle { x: 0, y: 0, width: geom.width, height: geom.height }],
            )?;
        }
        // Core-font text: no font dependency, and it is enough to tell the windows
        // apart in a screenshot. `image_text8` caps at 255 bytes.
        let line = format!(
            "{} {}{}",
            w.id,
            w.label,
            if w.override_redirect {
                "  [override-redirect]"
            } else if w.argb {
                "  [argb]"
            } else {
                ""
            }
        );
        let bytes: Vec<u8> = line.bytes().take(200).collect();
        let _ = self.conn.image_text8(w.window, text_gc, 8, 18, &bytes);
        let size = format!("{}x{}", geom.width, geom.height);
        let _ = self.conn.image_text8(w.window, text_gc, 8, 34, size.as_bytes());
        Ok(())
    }

    /// Returns `true` when the subject should exit.
    fn on_event(&mut self, event: Event) -> Result<bool, Box<dyn std::error::Error>> {
        match event {
            Event::Expose(e) => {
                if let Some(id) = self.id_of(e.window) {
                    self.redraw(id)?;
                    self.conn.flush()?;
                }
            }
            Event::ConfigureNotify(e) => {
                if let Some(id) = self.id_of(e.window) {
                    if let Some(w) = self.wins.get_mut(&id) {
                        let before = w.last_configure;
                        w.last_configure = (e.x, e.y, e.width, e.height);
                        if before != w.last_configure {
                            eprintln!(
                                "[subject] {id} configured by the compositor: {}x{}+{}+{} (was {}x{}+{}+{})",
                                e.width, e.height, e.x, e.y, before.2, before.3, before.0, before.1
                            );
                        }
                    }
                    self.redraw(id)?;
                    self.conn.flush()?;
                }
            }
            Event::ButtonPress(e) => {
                if let Some(id) = self.id_of(e.event) {
                    eprintln!("[subject] {id} CLICK at ({}, {}) — input reached this window", e.event_x, e.event_y);
                }
            }
            // A crossing is the earlier and more sensitive signal: it says which window
            // the X server's hit test picked, without needing a click to have been lost
            // before anyone notices.
            Event::EnterNotify(e) => {
                if let Some(id) = self.id_of(e.event) {
                    eprintln!(
                        "[subject] {id} ENTER at ({}, {}) — the server hit-tested to this window",
                        e.event_x, e.event_y
                    );
                }
            }
            Event::LeaveNotify(e) => {
                if let Some(id) = self.id_of(e.event) {
                    eprintln!("[subject] {id} LEAVE");
                }
            }
            // Throttled hard: motion is per-pixel and would bury everything else. One
            // line a second per window is enough to answer "is this window receiving
            // motion at all", which is the question.
            Event::MotionNotify(e) => {
                if let Some(id) = self.id_of(e.event) {
                    let now = std::time::Instant::now();
                    let due = self
                        .last_motion_log
                        .is_none_or(|at| now.duration_since(at).as_millis() >= 1000);
                    if due {
                        self.last_motion_log = Some(now);
                        eprintln!("[subject] {id} motion ({}, {})", e.event_x, e.event_y);
                    }
                }
            }
            Event::KeyPress(e) => {
                if let Some(id) = self.id_of(e.event) {
                    eprintln!("[subject] {id} KEY keycode={} — keyboard reached this window", e.detail);
                }
            }
            Event::ClientMessage(e) => {
                let data = e.data.as_data32();
                if e.type_ == self.atoms.xdnd_position {
                    // THE MEASUREMENT. The source hands us ROOT coordinates; a real target
                    // subtracts its own absolute position to find where in itself the
                    // pointer is, and lights up whatever drop zone that lands in.
                    //
                    // Under y5 every X11 toplevel sits at `shell::X11_ORIGIN`, so "our
                    // absolute position" is (0,0) however the canvas draws us — and the
                    // local position comes out equal to the root position. If that is
                    // outside our own geometry, no drop zone can ever be right, and the
                    // window drawn under the user's cursor is not the one being told about
                    // the drag.
                    let (rx, ry) = ((data[2] >> 16) as i16, (data[2] & 0xffff) as i16);
                    let abs = self.conn.translate_coordinates(e.window, self.root, 0, 0)?.reply()?;
                    let geom = self.conn.get_geometry(e.window)?.reply()?;
                    let (lx, ly) = (rx - abs.dst_x, ry - abs.dst_y);
                    let inside = lx >= 0 && ly >= 0 && lx < geom.width as i16 && ly < geom.height as i16;
                    eprintln!(
                        "[subject] {:?} dnd target: XdndPosition root=({rx},{ry}) my_abs=({},{}) \
                         -> local=({lx},{ly}) size={}x{} inside={inside}",
                        self.id_of(e.window),
                        abs.dst_x,
                        abs.dst_y,
                        geom.width,
                        geom.height,
                    );
                    // Accept, and ask to keep receiving positions (a zero rectangle means
                    // "no silent region", so the source keeps sending).
                    let reply = [e.window, 1, 0, 0, self.atoms.xdnd_action_copy];
                    self.send_xdnd(data[0], self.atoms.xdnd_status, reply)?;
                } else if e.type_ == self.atoms.xdnd_enter {
                    let version = data[1] >> 24;
                    eprintln!(
                        "[subject] {:?} dnd target: XdndEnter from {:#010x} (v{version})",
                        self.id_of(e.window),
                        data[0]
                    );
                } else if e.type_ == self.atoms.xdnd_leave {
                    eprintln!("[subject] {:?} dnd target: XdndLeave", self.id_of(e.window));
                } else if e.type_ == self.atoms.xdnd_drop {
                    eprintln!("[subject] {:?} dnd target: XdndDrop — accepting", self.id_of(e.window));
                    let reply = [e.window, 1, self.atoms.xdnd_action_copy, 0, 0];
                    self.send_xdnd(data[0], self.atoms.xdnd_finished, reply)?;
                } else if e.type_ == self.atoms.xdnd_status {
                    eprintln!(
                        "[subject] dnd source: XdndStatus from {:#010x} accept={}",
                        data[0],
                        data[1] & 1
                    );
                } else if e.type_ == self.atoms.xdnd_finished {
                    eprintln!("[subject] dnd source: XdndFinished from {:#010x}", data[0]);
                } else if e.type_ == self.atoms.wm_protocols {
                    if data[0] == self.atoms.wm_delete_window {
                        let id = self.id_of(e.window);
                        eprintln!("[subject] {id:?} WM_DELETE_WINDOW — closing politely");
                        self.conn.destroy_window(e.window)?;
                        if let Some(id) = id {
                            self.wins.remove(&id);
                        }
                        self.conn.flush()?;
                        if self.wins.is_empty() {
                            eprintln!("[subject] last window closed; exiting");
                            return Ok(true);
                        }
                    } else if data[0] == self.atoms.wm_take_focus {
                        // A globally-active client focuses ITSELF when told. Doing it
                        // is the whole point of the model — a compositor that never
                        // sends this leaves such a window unable to take the keyboard.
                        eprintln!("[subject] WM_TAKE_FOCUS for {:#x} — taking focus", e.window);
                        self.conn.set_input_focus(
                            InputFocus::PARENT,
                            e.window,
                            x11rb::CURRENT_TIME,
                        )?;
                        self.conn.flush()?;
                    }
                }
            }
            // Somebody (a wayland client, through the compositor's bridge) is reading
            // the selection we own.
            Event::SelectionRequest(e) => self.on_selection_request(e)?,
            // Our `paste` came back.
            Event::SelectionNotify(e) => {
                if e.property == x11rb::NONE {
                    eprintln!("[subject] paste: the selection owner refused or offered no UTF8_STRING");
                } else {
                    let reply = self
                        .conn
                        .get_property(true, e.requestor, e.property, AtomEnum::ANY, 0, u32::MAX)?
                        .reply()?;
                    eprintln!(
                        "[subject] paste: {:?}",
                        String::from_utf8_lossy(&reply.value)
                    );
                }
            }
            Event::SelectionClear(_) => {
                eprintln!("[subject] lost CLIPBOARD — a wayland client copied");
                self.clipboard = None;
            }
            Event::DestroyNotify(e) => {
                if let Some(id) = self.id_of(e.window) {
                    eprintln!("[subject] {id} destroyed by the server");
                    self.wins.remove(&id);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn on_selection_request(&self, e: SelectionRequestEvent) -> Result<(), Box<dyn std::error::Error>> {
        let mut property = e.property;
        match self.clipboard.as_ref() {
            Some(text) if e.target == self.atoms.utf8_string => {
                self.conn.change_property8(
                    PropMode::REPLACE,
                    e.requestor,
                    e.property,
                    self.atoms.utf8_string,
                    text.as_bytes(),
                )?;
                eprintln!("[subject] served CLIPBOARD as UTF8_STRING ({} bytes)", text.len());
            }
            Some(_) if e.target == self.atoms.targets => {
                self.conn.change_property32(
                    PropMode::REPLACE,
                    e.requestor,
                    e.property,
                    AtomEnum::ATOM,
                    &[self.atoms.targets, self.atoms.utf8_string],
                )?;
            }
            // Refusal is `property = None` in the notify, per ICCCM.
            _ => property = x11rb::NONE,
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property,
        };
        self.conn.send_event(false, e.requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }

    fn id_of(&self, window: Window) -> Option<u32> {
        self.wins.values().find(|w| w.window == window).map(|w| w.id)
    }
}

/// A 32-bit visual on this screen, if it offers one.
///
/// Every compositing X server does, Xwayland included, but it is a property of the screen
/// rather than a guarantee — so it is looked up and the caller degrades to an opaque
/// window rather than failing.
fn argb_visual(screen: &Screen) -> Option<Visualid> {
    screen
        .allowed_depths
        .iter()
        .find(|d| d.depth == 32)
        .and_then(|d| d.visuals.first())
        .map(|v| v.visual_id)
}

/// One `edge`-square ARGB image: a thick border in `outer`, a filled centre in `inner`.
///
/// TWO colours because the icon has to answer two questions at once. The border is the
/// window's own background colour, so an icon in a dock, an overview or a placeholder can
/// be matched back to the window it belongs to; the centre is the SIZE colour, which is
/// what says which image out of the property the compositor picked. Neither is legible
/// from the other, and a single-colour icon can only ever answer one of them.
///
/// The border is a sixth of the edge, floor 1px, so it survives at 16x16.
///
/// EWMH's pixel is a CARD32 with A in the high byte down to B in the low one — an
/// ARITHMETIC layout, not a memory one, so it is built by shifting and the machine's
/// endianness never enters into it. `change_property32` writes CARD32s, so the wire form
/// follows automatically.
fn framed(edge: u32, outer: u32, inner: u32) -> Vec<u32> {
    let border = (edge / 6).max(1);
    let mut v = Vec::with_capacity((edge * edge) as usize + 2);
    v.push(edge);
    v.push(edge);
    for y in 0..edge {
        for x in 0..edge {
            let edge_px = x < border || y < border || x + border >= edge || y + border >= edge;
            let rgb = if edge_px { outer } else { inner };
            v.push(0xff00_0000 | (rgb & 0x00ff_ffff));
        }
    }
    v
}

/// The colour an image of `edge` pixels is drawn in.
///
/// This is the whole measurement. The compositor picks ONE image out of the property and
/// shows it; the colour is how you read off which one it picked, with no instrumentation
/// on either side.
fn size_colour(edge: u32) -> u32 {
    match edge {
        16 => 0xd62828,   // red
        32 => 0xf77f00,   // orange
        48 => 0xfcbf49,   // yellow
        64 => 0x2a9d8f,   // green
        128 => 0x264653,  // blue
        256 => 0x7b2cbf,  // purple
        _ => 0xf5f5f5,    // white — the outsized ones
    }
}

/// The `_NET_WM_ICON` payload for a spec. Empty means "delete the property".
///
/// `window` is the owning window's background colour, painted as the border of every
/// image so the icon can be matched back to its window on sight — see [`framed`].
fn icon_words(spec: IconSpec, window: u32) -> Vec<u32> {
    let img = |edge: u32| framed(edge, window, size_colour(edge));
    match spec {
        IconSpec::None => Vec::new(),
        IconSpec::Single => img(64),
        // Ascending, so a decoder that simply takes the first or the last is visibly
        // wrong: 64 is GREEN, first is 16 (red) and last is 128 (blue).
        IconSpec::Pyramid => [16u32, 32, 48, 64, 128].iter().flat_map(|e| img(*e)).collect(),
        // Both within MAX_EDGE and neither at the preferred size or above except the
        // first, so the big one should win — and it is listed FIRST, which is the order
        // the decoder's own comment calls out.
        IconSpec::LargeFirst => {
            let mut v = img(1024);
            v.extend(img(48));
            v
        }
        // Over MAX_EDGE: must be skipped and the walk continued. 2048² would be 16 MiB
        // of words, more than the property is worth carrying, so the header lies about
        // a body we do not send — which ALSO makes this the truncation path if the size
        // guard is missing. Either way nothing after it is reachable unless the decoder
        // both skips and keeps walking.
        IconSpec::Oversize => {
            let mut v = vec![2048, 2048];
            v.extend(std::iter::repeat_n(0xff00_0000, 64));
            v.extend(img(48));
            v
        }
        // A good image, then a header claiming far more than follows. The good one must
        // survive.
        IconSpec::Truncated => {
            let mut v = img(64);
            v.extend([32, 32]);
            v.extend(std::iter::repeat_n(0xff00_0000, 10));
            v
        }
        // Zero dimensions end the walk, so the image AFTER it is unreachable by design.
        IconSpec::Zero => {
            let mut v = vec![0u32, 0];
            v.extend(img(64));
            v
        }
        // The WINDOW's colour, alpha ramping across the width — its own colour rather
        // than a neutral grey so this icon is identifiable like the rest. Straight alpha
        // (what the decoder assumes) fades evenly to nothing; premultiplied misread as
        // straight reads as a darkening toward the transparent edge.
        IconSpec::Alpha => {
            let edge = 64u32;
            let mut v = vec![edge, edge];
            for _y in 0..edge {
                for x in 0..edge {
                    let a = (x * 255 / (edge - 1)) & 0xff;
                    v.push((a << 24) | (window & 0x00ff_ffff));
                }
            }
            v
        }
    }
}
