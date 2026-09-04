//! Named scripted sequences, for repros and for checking one decision at a time.
//!
//! Each is a list of (command, pause-after) pairs the controller replays into the
//! subject. They exist because the interesting X11 cases are all *sequences* — a
//! window, then a menu on it, then the menu moving — and typing those by hand every
//! time is how a check quietly stops being run.
//!
//! Every scenario ends leaving its windows UP, so the screen can be looked at. Send
//! `quit` when done.

use crate::protocol::{CloseMode, Command, IconSpec, InputModel, WmType};
use std::time::Duration;

/// One step: what to send, and how long to wait before the next one.
pub struct Step(pub Command, pub Duration);

fn step(cmd: Command, ms: u64) -> Step {
    Step(cmd, Duration::from_millis(ms))
}

/// `(name, one-line description, steps)` for every scenario. The controller prints
/// this table for `--list`.
pub fn all() -> Vec<(&'static str, &'static str, Vec<Step>)> {
    vec![
        ("basics", "one managed window: title, class, resize, fullscreen", basics()),
        ("identity", "does the compositor see the APP's pid, not the X server's", identity()),
        ("focus", "all four ICCCM input models — the silent no-keyboard case", focus()),
        ("icon", "_NET_WM_ICON: which image does the decoder pick, and what survives a bad one", icon()),
        ("translucent", "a 32-bit ARGB window: is what is BEHIND it still drawn", translucent()),
        ("dnd", "XDND between two X11 windows: do the coordinates mean anything", dnd()),
        ("withdraw", "unmap/remap twice: two placeholders, one window, no abort", withdraw()),
        ("popup", "a REAL y5 popup: override-redirect + WM_TRANSIENT_FOR + menu type", popup()),
        ("popup-geometry", "a popup that moves and resizes ITSELF, the way a real menu does", popup_geometry()),
        ("popup-input", "does the popup get crossings and clicks where it overlaps its parent", popup_input()),
        ("types", "every _NET_WM_WINDOW_TYPE in turn: which get popup treatment", types()),
        ("type-flip", "the classification changes between map and surface association", type_flip()),
        ("self-geometry", "toplevel vs transient vs override-redirect moving/resizing themselves", self_geometry()),
        ("or-menu", "an override-redirect menu on a window: does it draw and take clicks", or_menu()),
        ("or-fight", "a client that re-anchors its own menu — the provisional decision's risk", or_fight()),
        ("or-submenu", "override-redirect chained off an override-redirect", or_submenu()),
        ("placement", "the compositor must honour a size request and refuse a move", placement()),
        ("transient", "WM_TRANSIENT_FOR — the window should size itself, not be locked", transient()),
        ("close", "WM_DELETE_WINDOW vs a window that does not offer it", close()),
        ("stacking", "two windows plus a menu: does the X server agree about the order", stacking()),
        ("clipboard", "X11 takes the clipboard, then reads it back", clipboard()),
        ("storm", "rapid resize + re-anchor: is the configure flush coalescing", storm()),
    ]
}

pub fn by_name(name: &str) -> Option<Vec<Step>> {
    all().into_iter().find(|(n, _, _)| *n == name).map(|(_, _, s)| s)
}

fn basics() -> Vec<Step> {
    vec![
        step(Command::Map(480, 320), 400),
        step(Command::Title(1, "x11 basics".into()), 200),
        step(Command::Class(1, "y5stress".into()), 200),
        step(Command::ResizeRequest(1, 640, 400), 600),
        step(Command::Fullscreen(1, true), 900),
        step(Command::Fullscreen(1, false), 600),
        step(Command::Info, 0),
    ]
}

/// The whole reason native XWayland was worth doing: under the satellite every X11
/// window reported the proxy's pid, so Steam attribution and the tearing tag never
/// fired. Check the compositor's window dump names THIS process.
fn identity() -> Vec<Step> {
    vec![
        step(Command::Map(400, 260), 400),
        step(Command::Title(1, "who owns me".into()), 200),
        step(Command::Class(1, "y5stress-identity".into()), 400),
        // Then compare against the compositor's own view — the subject prints its
        // real pid at startup, and `_NET_WM_PID` is what the compositor should be
        // reading rather than the surface credentials.
        step(Command::Info, 0),
    ]
}

/// Each model in turn, each on its own window, so they can be clicked between. A
/// window that takes no keyboard is the failure this exists to catch.
fn focus() -> Vec<Step> {
    vec![
        step(Command::Map(300, 200), 300),
        step(Command::Title(1, "passive".into()), 100),
        step(Command::InputModel(1, InputModel::Passive), 300),
        step(Command::Map(300, 200), 300),
        step(Command::Title(2, "locally-active".into()), 100),
        step(Command::InputModel(2, InputModel::LocallyActive), 300),
        step(Command::Map(300, 200), 300),
        step(Command::Title(3, "globally-active".into()), 100),
        step(Command::InputModel(3, InputModel::GloballyActive), 300),
        step(Command::Map(300, 200), 300),
        step(Command::Title(4, "no-input".into()), 100),
        step(Command::InputModel(4, InputModel::None), 300),
        step(Command::Info, 0),
    ]
}

/// A window with a NON-OPAQUE region, over an opaque one.
///
/// An X11 window on the default 24-bit visual has no alpha channel, so Xwayland can mark
/// its `wl_surface` fully opaque. A depth-32 window carries per-pixel alpha and cannot be
/// — and that opaque region is exactly what a compositor's occlusion culling reads. So
/// this is the arrangement that says whether y5 still draws what is BEHIND a translucent
/// X11 client.
///
/// Three outcomes, and they look different on purpose:
///
/// - correct: the ramp fades from the window beneath showing through on the left to solid
///   on the right, and the opaque frame marks the whole extent;
/// - culled: the frame is there and the left is BLACK or garbage rather than the window
///   beneath — the surface under it was skipped as occluded;
/// - alpha misread: the ramp washes out toward the transparent end instead of fading,
///   which is premultiplied colour being treated as straight.
///
/// The last two steps are the OTHER transparency, and unrelated: `_NET_WM_WINDOW_OPACITY`
/// on the opaque window is a compositing-manager convention applied to a window that has
/// no alpha channel at all. Ignoring it is a defensible answer; this just says which one
/// y5 gives.
fn translucent() -> Vec<Step> {
    vec![
        step(Command::Map(460, 320), 400),
        step(Command::Title(1, "opaque, behind".into()), 200),
        // Overlapping it, so most of the ARGB window has something to show through to.
        step(Command::MapArgb(420, 300), 700),
        step(Command::Title(2, "argb, in front".into()), 300),
        step(Command::Info, 300),
        // Move it across the window beneath: culling that is wrong only at rest is a
        // different bug from culling that is wrong while damage is moving.
        step(Command::SelfMove(2, 120, 90), 500),
        step(Command::SelfMove(2, 200, 150), 500),
        // And the unrelated property route, on the opaque window.
        step(Command::Opacity(1, 128), 700),
        step(Command::Opacity(1, 255), 0),
    ]
}

/// One window per `_NET_WM_ICON` shape, each set BEFORE its map — which is when
/// applications really set it and when the compositor resolves it.
///
/// The images are colour-coded by size, so the icon the compositor shows says which one
/// it chose without instrumenting either side: 16 red, 32 orange, 48 yellow, 64 green,
/// 128 blue, oversized white.
///
/// Expected, per window in order:
///
/// 1. `single`      — green. The plain case.
/// 2. `pyramid`     — GREEN, not red and not blue: the rule is the smallest image at
///                    least 64 across, so neither the first listed nor the largest.
/// 3. `large-first` — WHITE. 1024 is within the cap and 48 is under the preferred edge,
///                    so the "else the largest" arm takes the big one — listed first,
///                    which is the order the decoder's comment calls out.
/// 4. `oversize`    — YELLOW. 2048 is over the cap and must be SKIPPED with the walk
///                    continuing; a decoder that stops on it shows no icon at all.
/// 5. `truncated`   — GREEN. A header claiming more than follows ends the walk, and the
///                    good image already collected still counts.
/// 6. `zero`        — NO icon, falling back to the desktop entry. Zero dimensions end
///                    the walk, so the perfectly good image after them is unreachable
///                    by design. This one is here to prove that, not to pass.
/// 7. `alpha`       — a clean fade to transparent. Alpha is straight; premultiplied
///                    misread as straight darkens visibly toward the transparent edge.
///
/// The last window is the LATE case and the one most likely to disagree: the icon is set
/// after the map, and the compositor caches a window's icon in a `OnceLock`. If it has
/// already answered "no icon" for that window, a later property never shows up.
/// Two windows, one a drop TARGET and one a drag SOURCE, both logging the coordinates
/// they see.
///
/// XDND is the one X protocol where the CLIENT does the coordinate math, so it is the one
/// place stacking cannot rescue. The source reads the pointer with `query_pointer`, gets
/// ROOT coordinates, picks a window by walking the X tree, and sends those coordinates
/// unmodified; the target subtracts its own absolute position to decide which of its drop
/// zones is under the cursor. Neither side consults the window manager, and the protocol
/// has no room for one.
///
/// Under y5 every X11 toplevel sits at `shell::X11_ORIGIN` and the canvas draws it
/// somewhere else entirely, so watch three numbers in the log:
///
/// - `dnd source: XdndPosition root=(x,y)` — where the SOURCE thinks the pointer is;
/// - `dnd target: … my_abs=(ax,ay)` — where the TARGET thinks it is on screen, which will
///   be (0,0) for every toplevel;
/// - `-> local=(lx,ly) … inside=` — the position the target derives, and whether it even
///   falls within its own bounds.
///
/// If `inside=false` while the cursor is visibly over the target, that is the whole bug:
/// the message arrives at the right window and says the wrong place. If `inside=true` but
/// the numbers do not track where the cursor visibly is, the drop zones are being lit in
/// the wrong order for the same reason.
///
/// Run it, then drag the pointer across both windows before sending `dnd-drop`.
fn dnd() -> Vec<Step> {
    vec![
        step(Command::Map(420, 300), 400),
        step(Command::Title(1, "drag source".into()), 200),
        step(Command::Map(420, 300), 400),
        step(Command::Title(2, "drop target".into()), 200),
        step(Command::DndTarget(2), 200),
        // Make the source a target too: a pane torn off and dropped back into its own
        // window is the case that started this, and it needs both roles at once.
        step(Command::DndTarget(1), 200),
        step(Command::Info, 200),
        step(Command::DndSource(1), 0),
        // Now move the pointer over each window and read the three numbers above.
        // `dnd-drop` when done, or `dnd-cancel` to abandon.
    ]
}

/// Withdraw and remap the SAME window twice.
///
/// An X11 unmap is ICCCM's `Withdrawn` state — the window is gone from the screen but
/// still exists, and may be mapped again whenever the client likes. X11 gives no signal
/// separating that from a close, so y5 treats both the same way: the window leaves its
/// Space and a placeholder is left behind.
///
/// What this checks is the bookkeeping underneath, which has an assertion behind it. A
/// DESTROY moves the window's record into the placeholder, uuid and all; a WITHDRAWAL
/// copies it and leaves the original in place, so the window still has its own record
/// when it maps again. Get that wrong and the next move or resize of the readmitted
/// window reaches `PlaceholderState::modify`, which aborts on a missing record rather
/// than skipping — a compositor crash, not a glitch.
///
/// Expected: TWO placeholders (one per withdrawal, each with its own uuid), one live
/// window at the end, and no abort. The long first pause is deliberate — a placeholder is
/// refused for a window that lived under ten seconds, so a shorter wait would test
/// nothing.
fn withdraw() -> Vec<Step> {
    vec![
        step(Command::Map(420, 300), 500),
        step(Command::Title(1, "withdraw me".into()), 11_000),
        // First cycle.
        step(Command::Unmap(1), 2_000),
        step(Command::Remap(1), 2_000),
        // Second cycle: the copy carries the ORIGINAL record's age, so this one is past
        // the grace as well and must leave a placeholder of its own.
        step(Command::Unmap(1), 2_000),
        step(Command::Remap(1), 2_000),
        // Client-driven geometry on the readmitted window — the path that reads the
        // record back, and the one that aborts if the withdrawal moved it away.
        step(Command::SelfResize(1, 500, 360), 1_500),
        step(Command::Info, 0),
    ]
}

fn icon() -> Vec<Step> {
    let mut v = Vec::new();
    for spec in [
        IconSpec::Single,
        IconSpec::Pyramid,
        IconSpec::LargeFirst,
        IconSpec::Oversize,
        IconSpec::Truncated,
        IconSpec::Zero,
        IconSpec::Alpha,
    ] {
        v.push(step(Command::IconNext(spec), 50));
        v.push(step(Command::Map(260, 180), 300));
    }
    // Set after the map, on a window that mapped with none.
    v.push(step(Command::Map(260, 180), 400));
    v.push(step(Command::Icon(8, IconSpec::Pyramid), 600));
    v.push(step(Command::Info, 0));
    v
}

/// The canonical popup, which nothing in this harness could build before: an
/// override-redirect child that also names its parent and declares a menu type. All
/// THREE are needed — `child::as_popup` wants `is_ephemeral_x11` (which
/// override-redirect alone satisfies) plus a resolvable `WM_TRANSIENT_FOR`, and drop
/// either and it falls through to the ordinary window path.
///
/// Compare against `or-menu`, which is the same shape without the parent: that one is
/// a WINDOW to y5 — uuid, Space slot, decoration, placeholder — and this one is not.
fn popup() -> Vec<Step> {
    vec![
        step(Command::Map(520, 360), 500),
        step(Command::Title(1, "popup owner".into()), 200),
        // Inside the parent, like a context menu.
        step(Command::MapChild(1, 60, 40, 200, 160, true, WmType::Menu), 700),
        // And a submenu off the menu — the chain the xdg role test cannot see, which
        // is what `X11PopupLink` exists for. A submenu that resolves its root to the
        // menu above it instead of the toplevel gets a zero offset and lands wrong.
        step(Command::MapChild(2, 190, 30, 180, 140, true, WmType::PopupMenu), 700),
        step(Command::Info, 0),
    ]
}

/// A real menu does not sit still: it re-anchors when it would run off an edge, and it
/// resizes when its content changes. Both arrive as a plain `ConfigureWindow` that no
/// window manager is consulted about — so what y5 draws has to follow the server.
///
/// The last step is the one that catches a compositor handling x/y and w/h on separate
/// paths: one request carrying both.
fn popup_geometry() -> Vec<Step> {
    let mut v = vec![
        step(Command::Map(520, 360), 500),
        step(Command::MapChild(1, 60, 40, 200, 160, true, WmType::Menu), 600),
    ];
    // Walk it around, the way a menu re-anchoring off an edge does.
    for i in 1..=4 {
        v.push(step(Command::SelfMove(2, 80 + 40 * i, 60 + 30 * i), 400));
    }
    // Grow and shrink in place: content changing under an open menu.
    v.push(step(Command::SelfResize(2, 320, 260), 500));
    v.push(step(Command::SelfResize(2, 160, 120), 500));
    // Both at once.
    v.push(step(Command::SelfMoveResize(2, 120, 90, 280, 220), 500));
    v.push(step(Command::Info, 0));
    v
}

/// Where a popup overlaps its parent, which of them gets the input?
///
/// Move the pointer over the overlap by hand and read the `ENTER` / `CLICK` lines: they
/// name the window the X SERVER hit-tested to, which is the only thing that decides
/// where an ungrabbed event lands. `pointer` asks the server the same question directly,
/// and `stack` shows the order that produced the answer.
fn popup_input() -> Vec<Step> {
    vec![
        step(Command::Map(520, 360), 500),
        step(Command::Title(1, "input owner".into()), 200),
        // Deliberately well inside the parent, so every pixel of the popup is over it
        // and the two are genuinely competing for the same coordinates.
        step(Command::MapChild(1, 80, 60, 240, 200, true, WmType::Menu), 600),
        step(Command::Stack, 200),
        step(Command::Pointer, 200),
        step(Command::Info, 0),
        // Now move the pointer between the popup and the parent, and click each.
    ]
}

/// One child per type, so the classification can be read off the screen rather than
/// guessed at. Menu, popup-menu, dropdown-menu, tooltip, dnd and combo should be
/// popups; dialog, utility, toolbar, normal and dock should be windows.
///
/// `Unset` is last and is the interesting one: with no declared type the answer comes
/// from `skip_taskbar && !supports_delete_window`, so it is shown both ways.
fn types() -> Vec<Step> {
    let mut v = vec![
        step(Command::Map(560, 380), 500),
        step(Command::Title(1, "type owner".into()), 200),
    ];
    let mut n = 0i16;
    for t in WmType::all() {
        if t == WmType::Unset {
            continue;
        }
        n += 1;
        v.push(step(
            Command::MapChild(1, 20 * n, 16 * n, 150, 90, true, t),
            250,
        ));
    }
    v.push(step(Command::Info, 0));
    v
}

/// The classification is taken TWICE — once at the map request, again when the
/// wl_surface associates — and every input to it except override-redirect is an
/// ordinary property the client may change in between.
///
/// So: a managed child that looks like chrome at map (no type, skips the taskbar, no
/// `WM_DELETE_WINDOW`), then stops looking like it a moment later. If the two readings
/// disagree the window can end up queued twice, or never queued at all.
fn type_flip() -> Vec<Step> {
    vec![
        step(Command::Map(520, 360), 500),
        // Chrome-shaped at birth: no declared type is the arm that falls back to the
        // skip-taskbar heuristic.
        step(Command::MapChild(1, 60, 40, 220, 170, false, WmType::Unset), 100),
        step(Command::SkipTaskbar(2, true), 100),
        step(Command::CloseMode(2, CloseMode::NoDelete), 400),
        // ...and now it is an ordinary dialog after all.
        step(Command::WindowType(2, WmType::Dialog), 400),
        step(Command::CloseMode(2, CloseMode::Delete), 400),
        step(Command::SkipTaskbar(2, false), 400),
        // The reverse order, on a second child: ordinary at map, chrome afterwards.
        step(Command::MapChild(1, 200, 140, 220, 170, false, WmType::Normal), 400),
        step(Command::WindowType(3, WmType::Menu), 400),
        step(Command::TransientFor(3, 1), 400),
        step(Command::Info, 0),
    ]
}

/// The same self-configure against all three shapes, so the differences are side by
/// side: a plain toplevel, a transient dialog, and an override-redirect child.
///
/// Expected: every one keeps the SIZE it asks for, and only the override-redirect one
/// moves. A managed window's move is a `ConfigureRequest` y5 refuses — placement is the
/// compositor's, exactly as for an xdg toplevel.
fn self_geometry() -> Vec<Step> {
    vec![
        step(Command::Map(360, 260), 400),
        step(Command::Title(1, "toplevel".into()), 150),
        step(Command::MapTransient(1, 300, 220), 400),
        step(Command::Title(2, "transient".into()), 150),
        step(Command::MapChild(1, 60, 40, 200, 160, true, WmType::Menu), 400),
        // Sizes: all three should be taken.
        step(Command::SelfResize(1, 460, 320), 500),
        step(Command::SelfResize(2, 360, 260), 500),
        step(Command::SelfResize(3, 240, 190), 500),
        // Moves: only the override-redirect one may actually move.
        step(Command::SelfMove(1, 0, 0), 500),
        step(Command::SelfMove(2, 0, 0), 500),
        step(Command::SelfMove(3, 200, 150), 500),
        step(Command::Info, 0),
    ]
}

/// Override-redirect with NO parent — which y5 treats as a WINDOW, not a popup. Kept
/// separate from `popup` because it is a genuinely different path and a real one: X11
/// does not require a menu to set `WM_TRANSIENT_FOR`, and one that does not is the
/// known gap in the classification.
fn or_menu() -> Vec<Step> {
    vec![
        step(Command::Map(500, 340), 500),
        step(Command::Title(1, "menu owner".into()), 200),
        // Inside the parent, like a context menu...
        step(Command::Or(1, 60, 40, 200, 160), 800),
        // ...and one that overhangs the parent's edge, which is where a slot-cropped
        // window and an output-cropped popup behave differently.
        step(Command::Or(1, 430, 250, 220, 180), 0),
    ]
}

/// The case the override-redirect decision turns on. The client keeps moving its own
/// menu; y5 manages that window like any other, so the two are pulling against each
/// other. Watch whether the menu settles, jitters, or walks off.
fn or_fight() -> Vec<Step> {
    let mut v = vec![
        step(Command::Map(500, 340), 500),
        step(Command::Title(1, "re-anchor test".into()), 200),
        step(Command::Or(1, 60, 40, 200, 160), 600),
    ];
    for i in 1..=8 {
        v.push(step(Command::OrReanchor(2, 20 * i, 10 * i), 300));
    }
    v.push(step(Command::Info, 0));
    v
}

fn or_submenu() -> Vec<Step> {
    vec![
        step(Command::Map(520, 360), 500),
        step(Command::Or(1, 50, 40, 200, 160), 500),
        step(Command::OrChain(2, 190, 30), 500),
        step(Command::OrChain(3, 190, 30), 0),
    ]
}

fn placement() -> Vec<Step> {
    vec![
        step(Command::Map(400, 300), 500),
        step(Command::Title(1, "placement".into()), 200),
        // Honoured.
        step(Command::ResizeRequest(1, 700, 500), 700),
        // Refused — the window must NOT jump to these coordinates.
        step(Command::MoveRequest(1, 0, 0), 700),
        step(Command::MoveRequest(1, -400, -400), 700),
        step(Command::Info, 0),
    ]
}

fn transient() -> Vec<Step> {
    vec![
        step(Command::Map(520, 360), 500),
        step(Command::Title(1, "parent".into()), 200),
        step(Command::MapTransient(1, 260, 180), 500),
        step(Command::Title(2, "dialog".into()), 200),
        // A transient sizes itself, so the compositor should have left its slot
        // `Auto` — this resize should be taken at face value rather than snapped
        // back to whatever it first mapped at.
        step(Command::ResizeRequest(2, 320, 240), 600),
        step(Command::Info, 0),
    ]
}

fn close() -> Vec<Step> {
    vec![
        step(Command::Map(360, 240), 300),
        step(Command::Title(1, "offers WM_DELETE_WINDOW".into()), 100),
        step(Command::CloseMode(1, CloseMode::Delete), 300),
        step(Command::Map(360, 240), 300),
        step(Command::Title(2, "no WM_DELETE_WINDOW".into()), 100),
        step(Command::CloseMode(2, CloseMode::NoDelete), 300),
        step(Command::Info, 0),
        // Now close each from the compositor's own UI. Neither may take the X server
        // (or this process) down with it: the no-delete window is destroyed outright,
        // and escalating to the pid on top of that would kill the other window too.
    ]
}

fn stacking() -> Vec<Step> {
    vec![
        step(Command::Map(420, 300), 400),
        step(Command::Title(1, "lower".into()), 150),
        step(Command::Map(420, 300), 400),
        step(Command::Title(2, "upper".into()), 150),
        // A menu opened from the LOWER window. If the server disagrees with the
        // compositor about the order, it lands under the upper one.
        step(Command::Or(1, 40, 40, 220, 170), 600),
        step(Command::Stack, 300),
        step(Command::Raise(1), 600),
        step(Command::Stack, 0),
    ]
}

fn clipboard() -> Vec<Step> {
    vec![
        step(Command::Map(360, 240), 400),
        step(Command::Title(1, "clipboard".into()), 200),
        step(Command::Copy("hello from an x11 client".into()), 500),
        // Paste in a wayland app now; then copy there and run `paste` here.
        step(Command::Paste, 0),
    ]
}

fn storm() -> Vec<Step> {
    vec![
        step(Command::Map(500, 340), 500),
        step(Command::Or(1, 60, 40, 200, 160), 500),
        step(Command::StormResize(1, 300), 800),
        step(Command::StormReanchor(2, 300), 800),
        step(Command::Info, 0),
    ]
}
