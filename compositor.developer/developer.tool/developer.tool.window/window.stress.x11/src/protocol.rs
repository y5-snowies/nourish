//! The command vocabulary the controller sends to the subject (one command per stdin
//! line).
//!
//! Wire form is a compact, space-separated text line: a verb token followed by its
//! arguments, e.g. `or 1 40 20 200 120`, `input-model 1 globally`, `title 1 hello`.
//! Both processes share this module, so [`Command::encode`] and [`Command::parse`]
//! stay in sync with zero external deps.
//!
//! Window ids are the harness's own small integers (1, 2, 3 …), not X11 window ids —
//! the controller has to be able to name a window before the X server has told anyone
//! what its real id is.

use std::fmt::Write as _;

/// The ICCCM input model a window advertises, which is what decides how the
/// compositor must hand it the keyboard.
///
/// This is the single most silently-broken thing about an XWM: a compositor whose
/// seat focus is a plain `wl_surface` never reaches smithay's
/// `KeyboardTarget for X11Surface`, so unless it calls `set_input_focus` itself the X
/// server is never told who has focus and X clients get **no keyboard input at all**.
/// The four models take different paths through that call, so all four are worth
/// having in a harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputModel {
    /// `input = False`, no `WM_TAKE_FOCUS`. Never focused.
    None,
    /// `input = True`, no `WM_TAKE_FOCUS`. The WM calls `SetInputFocus`.
    Passive,
    /// `input = True` + `WM_TAKE_FOCUS`. Both.
    LocallyActive,
    /// `input = False` + `WM_TAKE_FOCUS`. The client focuses itself when told.
    GloballyActive,
}

impl InputModel {
    pub fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Passive => "passive",
            Self::LocallyActive => "locally",
            Self::GloballyActive => "globally",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "passive" => Self::Passive,
            "locally" => Self::LocallyActive,
            "globally" => Self::GloballyActive,
            _ => return None,
        })
    }
}

/// The `_NET_WM_WINDOW_TYPE` a window advertises.
///
/// The single most important property for y5's X11 classification and the one this
/// harness could not set at all before: `ident::is_popup_x11_surface` reads it first,
/// and a menu/tooltip/dnd/combo type is what turns an ephemeral child into a POPUP
/// (`PopupManager`, no uuid, no Space slot) rather than a window. Everything else is
/// taken at its word and stays a window.
///
/// `Unset` deletes the property, which is the interesting case rather than a gap: with
/// no declared type the classifier falls back to `skip_taskbar && !supports_delete`,
/// and that is the arm most likely to change under the client's feet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmType {
    Unset,
    Normal,
    Dialog,
    Menu,
    PopupMenu,
    DropdownMenu,
    Tooltip,
    Notification,
    Splash,
    Dnd,
    Combo,
    Utility,
    Toolbar,
    Dock,
}

impl WmType {
    pub fn token(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Normal => "normal",
            Self::Dialog => "dialog",
            Self::Menu => "menu",
            Self::PopupMenu => "popup-menu",
            Self::DropdownMenu => "dropdown-menu",
            Self::Tooltip => "tooltip",
            Self::Notification => "notification",
            Self::Splash => "splash",
            Self::Dnd => "dnd",
            Self::Combo => "combo",
            Self::Utility => "utility",
            Self::Toolbar => "toolbar",
            Self::Dock => "dock",
        }
    }

    /// The EWMH atom name, or `None` for [`Self::Unset`].
    pub fn atom_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Unset => return None,
            Self::Normal => "_NET_WM_WINDOW_TYPE_NORMAL",
            Self::Dialog => "_NET_WM_WINDOW_TYPE_DIALOG",
            Self::Menu => "_NET_WM_WINDOW_TYPE_MENU",
            Self::PopupMenu => "_NET_WM_WINDOW_TYPE_POPUP_MENU",
            Self::DropdownMenu => "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU",
            Self::Tooltip => "_NET_WM_WINDOW_TYPE_TOOLTIP",
            Self::Notification => "_NET_WM_WINDOW_TYPE_NOTIFICATION",
            Self::Splash => "_NET_WM_WINDOW_TYPE_SPLASH",
            Self::Dnd => "_NET_WM_WINDOW_TYPE_DND",
            Self::Combo => "_NET_WM_WINDOW_TYPE_COMBO",
            Self::Utility => "_NET_WM_WINDOW_TYPE_UTILITY",
            Self::Toolbar => "_NET_WM_WINDOW_TYPE_TOOLBAR",
            Self::Dock => "_NET_WM_WINDOW_TYPE_DOCK",
        })
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unset" => Self::Unset,
            "normal" => Self::Normal,
            "dialog" => Self::Dialog,
            "menu" => Self::Menu,
            "popup-menu" => Self::PopupMenu,
            "dropdown-menu" => Self::DropdownMenu,
            "tooltip" => Self::Tooltip,
            "notification" => Self::Notification,
            "splash" => Self::Splash,
            "dnd" => Self::Dnd,
            "combo" => Self::Combo,
            "utility" => Self::Utility,
            "toolbar" => Self::Toolbar,
            "dock" => Self::Dock,
            _ => return None,
        })
    }

    /// Every type, for the scenario that walks them one at a time.
    pub fn all() -> [WmType; 14] {
        [
            Self::Unset, Self::Normal, Self::Dialog, Self::Menu, Self::PopupMenu,
            Self::DropdownMenu, Self::Tooltip, Self::Notification, Self::Splash,
            Self::Dnd, Self::Combo, Self::Utility, Self::Toolbar, Self::Dock,
        ]
    }
}

/// A `_NET_WM_ICON` shape to publish — each one aimed at a branch of the compositor's
/// decoder rather than at looking nice.
///
/// EWMH packs images end to end as `width, height, width*height` CARD32 pixels, ARGB in
/// the arithmetic sense (A in the high byte). A property may carry several sizes in any
/// order, so the decoder has to walk them, pick one, and survive whatever a client got
/// wrong. Each variant is one of those situations.
///
/// The images are colour-coded BY SIZE, which is the whole trick: the compositor shows
/// one icon, and the colour is how you tell which image it chose without instrumenting
/// anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSpec {
    /// Delete the property. The window should fall back to its desktop-entry icon.
    None,
    /// One 64x64 image — the plain case.
    Single,
    /// 16, 32, 48, 64, 128 ascending. The documented rule is "smallest at least 64,
    /// else the largest", so 64 (GREEN) should win — not 128, and not the first listed.
    Pyramid,
    /// 1024 then 48. The comment in the decoder says a property is free to list 1024
    /// before 48; 1024 is within `MAX_EDGE`, so the big one (WHITE) should win on the
    /// "else the largest" arm — 48 is under the preferred edge.
    LargeFirst,
    /// 2048 then 48. Over `MAX_EDGE`, so the first image must be SKIPPED and the walk
    /// continue — 48 (YELLOW) wins. A decoder that stops on an outsized image shows
    /// nothing.
    Oversize,
    /// A header claiming more pixels than the property carries, after one good 64x64.
    /// The walk must stop and keep what it already has (GREEN), not discard everything.
    Truncated,
    /// A 0x0 header before a good image. Zero dimensions end the walk, so this one is
    /// expected to yield NOTHING — it is here to prove that is what happens rather
    /// than a panic.
    Zero,
    /// 64x64 with alpha ramping left to right over a fixed mid-grey RGB. Straight alpha
    /// (what the decoder assumes) fades cleanly; premultiplied misread as straight
    /// makes the transparent edge visibly darker.
    Alpha,
}

impl IconSpec {
    pub fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Single => "single",
            Self::Pyramid => "pyramid",
            Self::LargeFirst => "large-first",
            Self::Oversize => "oversize",
            Self::Truncated => "truncated",
            Self::Zero => "zero",
            Self::Alpha => "alpha",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "single" => Self::Single,
            "pyramid" => Self::Pyramid,
            "large-first" => Self::LargeFirst,
            "oversize" => Self::Oversize,
            "truncated" => Self::Truncated,
            "zero" => Self::Zero,
            "alpha" => Self::Alpha,
            _ => return None,
        })
    }
    pub fn all() -> [IconSpec; 8] {
        [
            Self::None, Self::Single, Self::Pyramid, Self::LargeFirst,
            Self::Oversize, Self::Truncated, Self::Zero, Self::Alpha,
        ]
    }
}

/// How a window answers a close request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    /// Lists `WM_DELETE_WINDOW`: the compositor asks politely and the client exits.
    Delete,
    /// Does NOT list it: a window manager destroys such a window outright. The
    /// compositor must NOT escalate to killing the pid on top of that — the pid is
    /// the whole subject process, and under a proxy it would have been the X server.
    NoDelete,
}

/// One controller -> subject instruction. See the module docs for the wire form.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // --- Managed windows ---
    /// Create a managed toplevel `w`x`h`.
    Map(u16, u16),
    /// A managed toplevel on a 32-bit ARGB visual — a window with a NON-OPAQUE region.
    ///
    /// The depth is the whole thing. An X11 window on the default 24-bit visual has no
    /// alpha channel at all, and Xwayland can mark its `wl_surface` fully opaque; a
    /// depth-32 window carries per-pixel alpha and cannot be. That opaque region is what
    /// a compositor's occlusion culling reads, so this is the window that says whether
    /// y5 still draws what is BEHIND a translucent X11 client, or culls it and shows
    /// black through the glass.
    ///
    /// Painted as vertical alpha bands, fully transparent on the left to fully opaque on
    /// the right, inside an opaque frame — so the window's extent stays visible even
    /// where its contents are not, and a wrong alpha interpretation (premultiplied vs
    /// straight) shows up as a colour shift across the ramp rather than as nothing.
    MapArgb(u16, u16),
    /// `_NET_WM_WINDOW_OPACITY` — the OTHER transparency, and a completely different
    /// mechanism: a whole-window CARD32 (`0xffffffff` opaque) that a compositing manager
    /// applies to an otherwise opaque window. Nothing to do with the visual's alpha
    /// channel, and honoured by convention rather than by any protocol. Worth having
    /// precisely because y5 may reasonably ignore it — this says which.
    Opacity(u32, u8),
    /// Create a managed toplevel with `WM_TRANSIENT_FOR` set to another window.
    /// The compositor should leave its expected size `Auto` (it sizes itself).
    MapTransient(u32, u16, u16),
    /// Unmap (WITHDRAW) a window without destroying it — ICCCM's `Withdrawn` state, and
    /// what a client does when it hides to a tray or minimises itself. The X window
    /// survives and can be mapped again.
    Unmap(u32),
    /// Map a withdrawn window again.
    Remap(u32),
    /// Destroy one of our windows from the client side.
    Destroy(u32),
    /// `WM_NAME` — where the compositor's window title comes from for X11.
    Title(u32, String),
    /// `WM_CLASS` — where `app_id` comes from for X11.
    Class(u32, String),
    /// `_NET_WM_PID`. Defaults to our real pid; set it to 0 to clear it and check
    /// what the compositor does with a window that will not say who owns it.
    Pid(u32, u32),
    /// `_NET_WM_STATE_FULLSCREEN` via a client message.
    Fullscreen(u32, bool),
    /// The ICCCM input model (`WM_HINTS` + `WM_PROTOCOLS`). See [`InputModel`].
    InputModel(u32, InputModel),
    /// Whether this window offers `WM_DELETE_WINDOW`. See [`CloseMode`].
    CloseMode(u32, CloseMode),

    // --- Classification properties (what y5 reads to decide popup vs window) ---
    /// `_NET_WM_WINDOW_TYPE`. See [`WmType`] — the FIRST thing the classifier reads.
    /// Settable after map on purpose: the classification is taken twice (at map and
    /// again at surface association) and the property can change in between.
    WindowType(u32, WmType),
    /// `_NET_WM_ICON` on an EXISTING window — after it mapped, which is the case the
    /// compositor's `OnceLock` icon cache may already have answered for.
    Icon(u32, IconSpec),
    /// The `_NET_WM_ICON` for the NEXT window created, written BEFORE its map request.
    /// This is what real applications do, and what the decoder's own comment assumes.
    IconNext(IconSpec),
    /// `_NET_WM_STATE_MODAL`. An ephemeral signal in its own right, independent of
    /// the window type.
    Modal(u32, bool),
    /// `_NET_WM_STATE_SKIP_TASKBAR`. Only consulted when NO window type is declared,
    /// and then only together with the absence of `WM_DELETE_WINDOW` — the classifier's
    /// weakest arm and the one worth poking at.
    SkipTaskbar(u32, bool),
    /// Set or clear `WM_TRANSIENT_FOR` after the fact (`0` clears it). An ephemeral
    /// window only becomes a POPUP if it names a resolvable parent, so this is the
    /// other half of the decision — and changing it late is how to check whether the
    /// compositor latched the answer or re-derives it.
    TransientFor(u32, u32),

    // --- Children (override-redirect, transient, or both) ---
    /// An override-redirect child of `parent`, placed at `parent + (dx, dy)`, with NO
    /// `WM_TRANSIENT_FOR`. y5 maps this as an ordinary window — a popup needs a
    /// resolvable parent, so this is deliberately the *other* path. See `MapChild`
    /// for one that can become a real popup.
    Or(u32, i16, i16, u16, u16),
    /// The general child: `parent + (dx, dy)`, `w`x`h`, override-redirect yes/no, and
    /// a window type — set BEFORE the map request, which is when y5 first classifies.
    /// `map-child 1 40 30 200 160 or menu` is the canonical y5 popup; swap `or` for
    /// `managed` and you have a transient dialog instead.
    MapChild(u32, i16, i16, u16, u16, bool, WmType),
    /// The client moves its OWN override-redirect window. This is the case that
    /// decides whether treating them as ordinary windows holds up: a client that
    /// re-anchors a menu is fighting the compositor's placement.
    OrReanchor(u32, i16, i16),
    /// An override-redirect child of an override-redirect window — a submenu.
    OrChain(u32, i16, i16),

    // --- Client-initiated geometry (the compositor should honour size, refuse position) ---
    /// `ConfigureRequest` asking for a new size.
    ResizeRequest(u32, u16, u16),
    /// `ConfigureRequest` asking to move. Must be REFUSED — placement is the
    /// compositor's, exactly as for an xdg toplevel.
    MoveRequest(u32, i16, i16),
    /// `ConfigureRequest` asking to restack.
    Raise(u32),
    Lower(u32),

    // --- Self-geometry: the same X request, two very different meanings ---
    //
    // For an override-redirect window a `ConfigureWindow` is not a request at all —
    // no window manager is consulted and the server just does it. For a managed
    // window the same call is redirected to the WM as a `ConfigureRequest`, which y5
    // honours for size and refuses for position. Issuing them through commands that
    // say which is expected is the point of these three.
    /// Move self to an ABSOLUTE root position.
    SelfMove(u32, i16, i16),
    /// Resize self, keeping position.
    SelfResize(u32, u16, u16),
    /// Move and resize in ONE request, which is what a menu re-anchoring actually
    /// sends and the case a compositor that handles x/y and w/h separately gets wrong.
    SelfMoveResize(u32, i16, i16, u16, u16),

    // --- XDND (drag and drop between X clients) ---
    //
    // The one protocol where the CLIENT does its own coordinate math, which is why it is
    // worth a harness of its own. A source sends `XdndPosition` in ROOT coordinates and
    // the target derives its own local position from them — so both sides are reasoning
    // about the X screen, while y5 keeps every toplevel at `shell::X11_ORIGIN` and draws
    // them wherever the canvas says. Stacking rescues ordinary pointer events; it cannot
    // rescue a message whose PAYLOAD is a coordinate.
    /// Make this window an XDND target (`XdndAware = 5`) and log every Xdnd message it
    /// receives, with the local position it would compute from each one.
    DndTarget(u32),
    /// Begin a drag from this window: take `XdndSelection`, then on every pump find the
    /// window under the pointer and send it `XdndEnter`/`XdndPosition`. Logs what it sees
    /// the pointer at and which window it picked, which is the source half of the same
    /// measurement.
    DndSource(u32),
    /// Finish the drag in progress with `XdndDrop`.
    DndDrop,
    /// Abandon the drag in progress with `XdndLeave`.
    DndCancel,

    // --- Selection bridge ---
    /// Take ownership of `CLIPBOARD` with this text, as an X client.
    Copy(String),
    /// Ask for `CLIPBOARD` and print what comes back — reads whatever a wayland
    /// client copied, through the compositor's bridge.
    Paste,

    // --- Storms ---
    /// `n` rapid resize requests. The compositor's per-frame configure flush should
    /// coalesce its replies rather than answering each one.
    StormResize(u32, u32),
    /// `n` rapid override-redirect re-anchors.
    StormReanchor(u32, u32),

    // --- Introspection ---
    /// Print every window the subject owns: harness id, X11 id, geometry as the
    /// SERVER has it, override-redirect flag, and the last configure we were sent.
    Info,
    /// Print the server's stacking order for our windows — the check on the
    /// compositor mirroring its own order into the X server.
    Stack,
    /// Ask the SERVER where the pointer is and which window it is in. The direct read
    /// on y5's stacking mechanism: an ungrabbed pointer event reaches whichever client
    /// the X server's own hit test lands on, so if this names the wrong window, that
    /// is why input went to the wrong place.
    Pointer,
    Quit,
}

impl Command {
    /// The wire form. Round-trips through [`Self::parse`].
    pub fn encode(&self) -> String {
        let mut s = String::new();
        match self {
            Self::Map(w, h) => write!(s, "map {w} {h}"),
            Self::MapArgb(w, h) => write!(s, "map-argb {w} {h}"),
            Self::Opacity(id, v) => write!(s, "opacity {id} {v}"),
            Self::MapTransient(id, w, h) => write!(s, "map-transient {id} {w} {h}"),
            Self::Unmap(id) => write!(s, "unmap {id}"),
            Self::Remap(id) => write!(s, "remap {id}"),
            Self::Destroy(id) => write!(s, "destroy {id}"),
            Self::Title(id, t) => write!(s, "title {id} {t}"),
            Self::Class(id, c) => write!(s, "class {id} {c}"),
            Self::Pid(id, p) => write!(s, "pid {id} {p}"),
            Self::Fullscreen(id, on) => write!(s, "fullscreen {id} {}", onoff(*on)),
            Self::InputModel(id, m) => write!(s, "input-model {id} {}", m.token()),
            Self::CloseMode(id, m) => write!(
                s,
                "close-mode {id} {}",
                match m {
                    CloseMode::Delete => "delete",
                    CloseMode::NoDelete => "nodelete",
                }
            ),
            Self::WindowType(id, t) => write!(s, "window-type {id} {}", t.token()),
            Self::Icon(id, spec) => write!(s, "icon {id} {}", spec.token()),
            Self::IconNext(spec) => write!(s, "icon-next {}", spec.token()),
            Self::Modal(id, on) => write!(s, "modal {id} {}", onoff(*on)),
            Self::SkipTaskbar(id, on) => write!(s, "skip-taskbar {id} {}", onoff(*on)),
            Self::TransientFor(id, p) => write!(s, "transient-for {id} {p}"),
            Self::Or(id, dx, dy, w, h) => write!(s, "or {id} {dx} {dy} {w} {h}"),
            Self::MapChild(id, dx, dy, w, h, or, t) => write!(
                s,
                "map-child {id} {dx} {dy} {w} {h} {} {}",
                if *or { "or" } else { "managed" },
                t.token()
            ),
            Self::OrReanchor(id, dx, dy) => write!(s, "or-reanchor {id} {dx} {dy}"),
            Self::OrChain(id, dx, dy) => write!(s, "or-chain {id} {dx} {dy}"),
            Self::ResizeRequest(id, w, h) => write!(s, "resize-request {id} {w} {h}"),
            Self::MoveRequest(id, x, y) => write!(s, "move-request {id} {x} {y}"),
            Self::Raise(id) => write!(s, "raise {id}"),
            Self::Lower(id) => write!(s, "lower {id}"),
            Self::SelfMove(id, x, y) => write!(s, "self-move {id} {x} {y}"),
            Self::SelfResize(id, w, h) => write!(s, "self-resize {id} {w} {h}"),
            Self::SelfMoveResize(id, x, y, w, h) => {
                write!(s, "self-move-resize {id} {x} {y} {w} {h}")
            }
            Self::DndTarget(id) => write!(s, "dnd-target {id}"),
            Self::DndSource(id) => write!(s, "dnd-source {id}"),
            Self::DndDrop => write!(s, "dnd-drop"),
            Self::DndCancel => write!(s, "dnd-cancel"),
            Self::Copy(t) => write!(s, "copy {t}"),
            Self::Paste => write!(s, "paste"),
            Self::StormResize(id, n) => write!(s, "storm-resize {id} {n}"),
            Self::StormReanchor(id, n) => write!(s, "storm-reanchor {id} {n}"),
            Self::Info => write!(s, "info"),
            Self::Stack => write!(s, "stack"),
            Self::Pointer => write!(s, "pointer"),
            Self::Quit => write!(s, "quit"),
        }
        .expect("writing to a String cannot fail");
        s
    }

    /// Parse one wire line. `None` for an unknown verb or a bad argument — the
    /// subject reports and carries on rather than exiting, so a typo in an
    /// interactive session costs nothing.
    pub fn parse(line: &str) -> Option<Self> {
        let mut it = line.split_whitespace();
        let verb = it.next()?;
        // The text-carrying verbs take the REST of the line verbatim, spaces and all.
        let rest = |it: &mut std::str::SplitWhitespace| -> String {
            it.collect::<Vec<_>>().join(" ")
        };
        Some(match verb {
            "map" => Self::Map(num(it.next())?, num(it.next())?),
            "map-argb" => Self::MapArgb(num(it.next())?, num(it.next())?),
            "opacity" => Self::Opacity(num(it.next())?, num(it.next())?),
            "map-transient" => Self::MapTransient(num(it.next())?, num(it.next())?, num(it.next())?),
            "unmap" => Self::Unmap(num(it.next())?),
            "remap" => Self::Remap(num(it.next())?),
            "destroy" => Self::Destroy(num(it.next())?),
            "title" => {
                let id = num(it.next())?;
                Self::Title(id, rest(&mut it))
            }
            "class" => {
                let id = num(it.next())?;
                Self::Class(id, rest(&mut it))
            }
            "pid" => Self::Pid(num(it.next())?, num(it.next())?),
            "fullscreen" => Self::Fullscreen(num(it.next())?, flag(it.next())?),
            "input-model" => Self::InputModel(num(it.next())?, InputModel::parse(it.next()?)?),
            "close-mode" => Self::CloseMode(
                num(it.next())?,
                match it.next()? {
                    "delete" => CloseMode::Delete,
                    "nodelete" => CloseMode::NoDelete,
                    _ => return None,
                },
            ),
            "window-type" => Self::WindowType(num(it.next())?, WmType::parse(it.next()?)?),
            "icon" => Self::Icon(num(it.next())?, IconSpec::parse(it.next()?)?),
            "icon-next" => Self::IconNext(IconSpec::parse(it.next()?)?),
            "modal" => Self::Modal(num(it.next())?, flag(it.next())?),
            "skip-taskbar" => Self::SkipTaskbar(num(it.next())?, flag(it.next())?),
            "transient-for" => Self::TransientFor(num(it.next())?, num(it.next())?),
            "map-child" => Self::MapChild(
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                match it.next()? {
                    "or" | "override-redirect" => true,
                    "managed" => false,
                    _ => return None,
                },
                WmType::parse(it.next()?)?,
            ),
            "or" => Self::Or(
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
            ),
            "or-reanchor" => Self::OrReanchor(num(it.next())?, num(it.next())?, num(it.next())?),
            "or-chain" => Self::OrChain(num(it.next())?, num(it.next())?, num(it.next())?),
            "resize-request" => Self::ResizeRequest(num(it.next())?, num(it.next())?, num(it.next())?),
            "move-request" => Self::MoveRequest(num(it.next())?, num(it.next())?, num(it.next())?),
            "raise" => Self::Raise(num(it.next())?),
            "lower" => Self::Lower(num(it.next())?),
            "self-move" => Self::SelfMove(num(it.next())?, num(it.next())?, num(it.next())?),
            "self-resize" => Self::SelfResize(num(it.next())?, num(it.next())?, num(it.next())?),
            "self-move-resize" => Self::SelfMoveResize(
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
                num(it.next())?,
            ),
            "dnd-target" => Self::DndTarget(num(it.next())?),
            "dnd-source" => Self::DndSource(num(it.next())?),
            "dnd-drop" => Self::DndDrop,
            "dnd-cancel" => Self::DndCancel,
            "copy" => Self::Copy(rest(&mut it)),
            "paste" => Self::Paste,
            "storm-resize" => Self::StormResize(num(it.next())?, num(it.next())?),
            "storm-reanchor" => Self::StormReanchor(num(it.next())?, num(it.next())?),
            "info" => Self::Info,
            "stack" => Self::Stack,
            "pointer" => Self::Pointer,
            "quit" | "exit" => Self::Quit,
            _ => return None,
        })
    }
}

fn num<T: std::str::FromStr>(tok: Option<&str>) -> Option<T> {
    tok?.parse().ok()
}

fn flag(tok: Option<&str>) -> Option<bool> {
    Some(match tok? {
        "on" | "true" | "1" => true,
        "off" | "false" | "0" => false,
        _ => return None,
    })
}

fn onoff(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

/// Every command, once each, with arguments that exercise the parser's shapes.
///
/// Drives the controller's `--selftest`: the whole vocabulary encodes and parses back
/// without an X server, a compositor or a subject process, so a typo in the table is
/// caught by running one binary rather than by a session that silently ignores a line.
pub fn all_commands() -> Vec<Command> {
    use Command::*;
    vec![
        Map(400, 300),
        MapArgb(420, 300),
        Opacity(1, 128),
        Opacity(1, 255),
        MapTransient(1, 200, 150),
        Unmap(1),
        Remap(1),
        Destroy(2),
        Title(1, "a window with spaces".into()),
        Class(1, "y5stress".into()),
        Pid(1, 4242),
        Fullscreen(1, true),
        Fullscreen(1, false),
        InputModel(1, self::InputModel::None),
        InputModel(1, self::InputModel::Passive),
        InputModel(1, self::InputModel::LocallyActive),
        InputModel(1, self::InputModel::GloballyActive),
        CloseMode(1, self::CloseMode::Delete),
        CloseMode(1, self::CloseMode::NoDelete),
        WindowType(1, WmType::Unset),
        WindowType(1, WmType::Menu),
        WindowType(1, WmType::Tooltip),
        WindowType(1, WmType::Dialog),
        WindowType(1, WmType::Normal),
        Icon(1, IconSpec::Pyramid),
        Icon(1, IconSpec::None),
        IconNext(IconSpec::Single),
        IconNext(IconSpec::Alpha),
        Modal(1, true),
        Modal(1, false),
        SkipTaskbar(1, true),
        SkipTaskbar(1, false),
        TransientFor(2, 1),
        TransientFor(2, 0),
        Or(1, 40, 20, 180, 120),
        Or(1, -40, -20, 180, 120),
        MapChild(1, 40, 30, 200, 160, true, WmType::Menu),
        MapChild(1, 40, 30, 200, 160, false, WmType::Dialog),
        OrReanchor(2, 10, 10),
        OrChain(2, 30, 30),
        ResizeRequest(1, 640, 480),
        MoveRequest(1, -100, 50),
        Raise(1),
        Lower(1),
        SelfMove(2, 120, 90),
        SelfResize(2, 240, 200),
        SelfMoveResize(2, -60, 40, 260, 210),
        DndTarget(2),
        DndSource(1),
        DndDrop,
        DndCancel,
        Copy("clipboard text from x11".into()),
        Paste,
        StormResize(1, 200),
        StormReanchor(2, 200),
        Info,
        Stack,
        Pointer,
        Quit,
    ]
}
