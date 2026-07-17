//! The command vocabulary the controller sends to the subject (one command per stdin line).
//!
//! Wire form is a compact, space-separated text line: a verb token followed by its arguments,
//! e.g. `layer overlay`, `excl 40`, `margin 8 8 8 8`, `popup-off 40 0`. Both processes share
//! this module, so [`Command::encode`] and [`Command::parse`] stay in sync with zero external
//! deps.

use std::fmt::Write as _;

/// wlr layer level a surface sits on (`zwlr_layer_shell_v1` layer arg).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

/// Keyboard interactivity mode (`zwlr_layer_surface_v1.set_keyboard_interactivity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kbd {
    None,
    Exclusive,
    OnDemand,
}

/// Input-region shape requested via `wl_surface.set_input_region`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Full,   // whole surface accepts input (default)
    None,   // empty region -> click-through
    Circle, // rectangle-approximated disc in the centre
    Holes,  // full region minus an inner rectangle
}

/// Buffer-scale behaviour (a trimmed subset of the window harness's scale modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleReq {
    Normal,
    FsHonor,
    FsIgnore,
    DpiHonor,
    DpiIgnore,
    DpiScale(i32),
}

/// A control request issued on a selected wlr foreign-toplevel handle
/// (`zwlr_foreign_toplevel_handle_v1`). The ext list protocol is read-only, so it has no
/// analog — foreign control exercises the wlr interface's request set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignCtl {
    Activate,
    Close,
    Maximized(bool),
    Minimized(bool),
    Fullscreen(bool),
}

/// xdg_positioner anchor point / gravity, reused for popups spawned from the layer surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
pub type Gravity = Anchor;

/// One controller -> subject instruction. See the module docs for the wire form.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // --- Layer level ---
    SetLayer(Layer),

    // --- Anchor (edge toggles + presets) ---
    AnchorTop,
    AnchorBottom,
    AnchorLeft,
    AnchorRight,
    AnchorAll,
    AnchorNone,

    // --- Exclusive zone ---
    Excl(i32),

    // --- Margins (top, right, bottom, left) ---
    Margin(i32, i32, i32, i32),

    // --- Keyboard interactivity ---
    Keyboard(Kbd),

    // --- Size (0 in a dimension = stretch along the anchored edge) ---
    Size(u32, u32),

    // --- Popup (from the layer surface) ---
    PopupAdd,
    PopupClose,
    PopupAnchor(Anchor),
    PopupGravity(Gravity),
    PopupOff(i32, i32),
    PopupSize(u32, u32),
    PopupMove(i32, i32),

    // --- Input / opaque regions ---
    Input(Region),
    Opaque(bool), // true = whole surface opaque, false = unset

    // --- Output binding ---
    OutputCycle,

    // --- Buffer scale ---
    Scale(ScaleReq),

    // --- Appearance ---
    Transparent(bool),
    Animate(bool),

    // --- Foreign toplevel control (wlr manager) ---
    ForeignSel(i32), // cycle the selected foreign toplevel by +1 / -1
    Foreign(ForeignCtl),

    // --- Lifecycle ---
    Recreate,
    Quit,
}

impl Anchor {
    fn token(self) -> &'static str {
        match self {
            Anchor::Center => "c",
            Anchor::Top => "t",
            Anchor::Bottom => "b",
            Anchor::Left => "l",
            Anchor::Right => "r",
            Anchor::TopLeft => "tl",
            Anchor::TopRight => "tr",
            Anchor::BottomLeft => "bl",
            Anchor::BottomRight => "br",
        }
    }
    fn parse(s: &str) -> Option<Anchor> {
        Some(match s {
            "c" => Anchor::Center,
            "t" => Anchor::Top,
            "b" => Anchor::Bottom,
            "l" => Anchor::Left,
            "r" => Anchor::Right,
            "tl" => Anchor::TopLeft,
            "tr" => Anchor::TopRight,
            "bl" => Anchor::BottomLeft,
            "br" => Anchor::BottomRight,
            _ => return None,
        })
    }
}

impl Command {
    /// Render to a single wire line (no trailing newline).
    pub fn encode(&self) -> String {
        let mut s = String::new();
        match self {
            Command::SetLayer(l) => {
                let _ = write!(s, "layer {}", layer_tok(*l));
            }
            Command::AnchorTop => s.push_str("anchor-top"),
            Command::AnchorBottom => s.push_str("anchor-bottom"),
            Command::AnchorLeft => s.push_str("anchor-left"),
            Command::AnchorRight => s.push_str("anchor-right"),
            Command::AnchorAll => s.push_str("anchor-all"),
            Command::AnchorNone => s.push_str("anchor-none"),

            Command::Excl(n) => {
                let _ = write!(s, "excl {n}");
            }
            Command::Margin(t, r, b, l) => {
                let _ = write!(s, "margin {t} {r} {b} {l}");
            }
            Command::Keyboard(k) => {
                let _ = write!(s, "kbd {}", kbd_tok(*k));
            }
            Command::Size(w, h) => {
                let _ = write!(s, "size {w} {h}");
            }

            Command::PopupAdd => s.push_str("popup-add"),
            Command::PopupClose => s.push_str("popup-close"),
            Command::PopupAnchor(a) => {
                let _ = write!(s, "popup-anchor {}", a.token());
            }
            Command::PopupGravity(g) => {
                let _ = write!(s, "popup-gravity {}", g.token());
            }
            Command::PopupOff(x, y) => {
                let _ = write!(s, "popup-off {x} {y}");
            }
            Command::PopupSize(w, h) => {
                let _ = write!(s, "popup-size {w} {h}");
            }
            Command::PopupMove(x, y) => {
                let _ = write!(s, "popup-move {x} {y}");
            }

            Command::Input(r) => {
                let _ = write!(s, "input {}", region_tok(*r));
            }
            Command::Opaque(on) => {
                let _ = write!(s, "opaque {}", if *on { "full" } else { "none" });
            }

            Command::OutputCycle => s.push_str("output-cycle"),

            Command::Scale(sr) => s.push_str(scale_tok(*sr).as_str()),

            Command::Transparent(on) => {
                let _ = write!(s, "transparent {}", on8(*on));
            }
            Command::Animate(on) => {
                let _ = write!(s, "animate {}", on8(*on));
            }

            Command::ForeignSel(d) => {
                let _ = write!(s, "foreign-sel {d}");
            }
            Command::Foreign(c) => match c {
                ForeignCtl::Activate => s.push_str("foreign-activate"),
                ForeignCtl::Close => s.push_str("foreign-close"),
                ForeignCtl::Maximized(on) => {
                    let _ = write!(s, "foreign-max {}", on8(*on));
                }
                ForeignCtl::Minimized(on) => {
                    let _ = write!(s, "foreign-min {}", on8(*on));
                }
                ForeignCtl::Fullscreen(on) => {
                    let _ = write!(s, "foreign-full {}", on8(*on));
                }
            },

            Command::Recreate => s.push_str("recreate"),
            Command::Quit => s.push_str("quit"),
        }
        s
    }

    /// Parse a single wire line, ignoring surrounding whitespace. Returns `None` for an
    /// unknown verb or malformed args.
    pub fn parse(line: &str) -> Option<Command> {
        let mut it = line.split_whitespace();
        let verb = it.next()?;
        let mut next = || it.next();
        Some(match verb {
            "layer" => Command::SetLayer(match next()? {
                "background" => Layer::Background,
                "bottom" => Layer::Bottom,
                "top" => Layer::Top,
                "overlay" => Layer::Overlay,
                _ => return None,
            }),
            "anchor-top" => Command::AnchorTop,
            "anchor-bottom" => Command::AnchorBottom,
            "anchor-left" => Command::AnchorLeft,
            "anchor-right" => Command::AnchorRight,
            "anchor-all" => Command::AnchorAll,
            "anchor-none" => Command::AnchorNone,

            "excl" => Command::Excl(next()?.parse().ok()?),
            "margin" => Command::Margin(
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
            ),
            "kbd" => Command::Keyboard(match next()? {
                "none" => Kbd::None,
                "exclusive" => Kbd::Exclusive,
                "ondemand" => Kbd::OnDemand,
                _ => return None,
            }),
            "size" => Command::Size(next()?.parse().ok()?, next()?.parse().ok()?),

            "popup-add" => Command::PopupAdd,
            "popup-close" => Command::PopupClose,
            "popup-anchor" => Command::PopupAnchor(Anchor::parse(next()?)?),
            "popup-gravity" => Command::PopupGravity(Anchor::parse(next()?)?),
            "popup-off" => Command::PopupOff(next()?.parse().ok()?, next()?.parse().ok()?),
            "popup-size" => Command::PopupSize(next()?.parse().ok()?, next()?.parse().ok()?),
            "popup-move" => Command::PopupMove(next()?.parse().ok()?, next()?.parse().ok()?),

            "input" => Command::Input(match next()? {
                "full" => Region::Full,
                "none" => Region::None,
                "circle" => Region::Circle,
                "holes" => Region::Holes,
                _ => return None,
            }),
            "opaque" => Command::Opaque(match next()? {
                "full" => true,
                "none" => false,
                _ => return None,
            }),

            "output-cycle" => Command::OutputCycle,

            "scale-normal" => Command::Scale(ScaleReq::Normal),
            "fs-honor" => Command::Scale(ScaleReq::FsHonor),
            "fs-ignore" => Command::Scale(ScaleReq::FsIgnore),
            "dpi-honor" => Command::Scale(ScaleReq::DpiHonor),
            "dpi-ignore" => Command::Scale(ScaleReq::DpiIgnore),
            "dpi-scale" => Command::Scale(ScaleReq::DpiScale(next()?.parse().ok()?)),

            "transparent" => Command::Transparent(parse_on(next()?)?),
            "animate" => Command::Animate(parse_on(next()?)?),

            "foreign-sel" => Command::ForeignSel(next()?.parse().ok()?),
            "foreign-activate" => Command::Foreign(ForeignCtl::Activate),
            "foreign-close" => Command::Foreign(ForeignCtl::Close),
            "foreign-max" => Command::Foreign(ForeignCtl::Maximized(parse_on(next()?)?)),
            "foreign-min" => Command::Foreign(ForeignCtl::Minimized(parse_on(next()?)?)),
            "foreign-full" => Command::Foreign(ForeignCtl::Fullscreen(parse_on(next()?)?)),

            "recreate" => Command::Recreate,
            "quit" => Command::Quit,

            _ => return None,
        })
    }
}

fn layer_tok(l: Layer) -> &'static str {
    match l {
        Layer::Background => "background",
        Layer::Bottom => "bottom",
        Layer::Top => "top",
        Layer::Overlay => "overlay",
    }
}

fn kbd_tok(k: Kbd) -> &'static str {
    match k {
        Kbd::None => "none",
        Kbd::Exclusive => "exclusive",
        Kbd::OnDemand => "ondemand",
    }
}

fn region_tok(r: Region) -> &'static str {
    match r {
        Region::Full => "full",
        Region::None => "none",
        Region::Circle => "circle",
        Region::Holes => "holes",
    }
}

fn scale_tok(s: ScaleReq) -> String {
    match s {
        ScaleReq::Normal => "scale-normal".into(),
        ScaleReq::FsHonor => "fs-honor".into(),
        ScaleReq::FsIgnore => "fs-ignore".into(),
        ScaleReq::DpiHonor => "dpi-honor".into(),
        ScaleReq::DpiIgnore => "dpi-ignore".into(),
        ScaleReq::DpiScale(n) => format!("dpi-scale {n}"),
    }
}

fn on8(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

fn parse_on(s: &str) -> Option<bool> {
    match s {
        "on" | "1" | "true" => Some(true),
        "off" | "0" | "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cases = [
            Command::SetLayer(Layer::Overlay),
            Command::AnchorTop,
            Command::AnchorAll,
            Command::AnchorNone,
            Command::Excl(-1),
            Command::Margin(8, 4, 8, 4),
            Command::Keyboard(Kbd::OnDemand),
            Command::Size(300, 0),
            Command::PopupOff(40, 0),
            Command::PopupAnchor(Anchor::BottomRight),
            Command::Input(Region::Holes),
            Command::Opaque(true),
            Command::OutputCycle,
            Command::Scale(ScaleReq::DpiScale(2)),
            Command::Scale(ScaleReq::FsHonor),
            Command::Transparent(true),
            Command::Animate(false),
            Command::ForeignSel(-1),
            Command::Foreign(ForeignCtl::Activate),
            Command::Foreign(ForeignCtl::Maximized(true)),
            Command::Foreign(ForeignCtl::Fullscreen(false)),
            Command::Recreate,
            Command::Quit,
        ];
        for c in cases {
            let line = c.encode();
            assert_eq!(Command::parse(&line), Some(c.clone()), "line = {line:?}");
        }
    }
}
