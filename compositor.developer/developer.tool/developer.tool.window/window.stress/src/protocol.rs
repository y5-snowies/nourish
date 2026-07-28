//! The command vocabulary the controller sends to the subject (one command per stdin line).
//!
//! Wire form is a compact, space-separated text line: a verb token followed by its
//! arguments, e.g. `popup-off 400 0`, `sp-fill 255 0 0 255`, `deco-mode server`. Both
//! processes share this module, so [`Command::encode`] and [`Command::parse`] stay in sync
//! with zero external deps.

use std::fmt::Write as _;

/// Server/client/none decoration mode requested via `zxdg_toplevel_decoration_v1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoMode {
    Server,
    Client,
    None,
}

/// xdg_positioner anchor point on the parent rectangle.
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

/// xdg_positioner gravity (direction the popup expands from the anchor).
pub type Gravity = Anchor;

/// One controller -> subject instruction. See the module docs for the wire form.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // --- Decoration ---
    DecoMode(DecoMode),
    DecoIgnore, // toggle: keep drawing our chosen chrome regardless of compositor mode
    DecoBadsize, // toggle: draw CSD chrome whose thickness disagrees with window geometry

    // --- Buffer size ---
    BufAgreed,
    BufDelta(i32),
    BufZero,
    BufNoack,
    BufPreack,
    GeoMismatch,

    // --- Popup ---
    PopupAdd,
    PopupAnchor(Anchor),
    PopupGravity(Gravity),
    PopupOff(i32, i32),
    PopupSize(u32, u32),
    PopupMove(i32, i32),
    PopupNest,
    PopupClose,

    // --- Subsurface ---
    SubAdd,
    SubMove(i32, i32),
    SubSync,
    SubDesync,
    SubNest,
    SubRemove,

    // --- Viewporter ---
    VpDest(i32, i32),
    VpDestDelta(i32),
    VpSrc(f64, f64, f64, f64),
    VpAnimate(bool),
    VpUnset,
    VpBad,

    // --- Fractional scale ---
    FsHonor,
    FsIgnore, // toggle
    FsScale(u32), // numerator of n/120
    FsNoViewport,
    FsMismatch,

    // --- DPI / integer scale ---
    DpiHonor,
    DpiIgnore, // toggle
    DpiScale(i32),
    DpiNondiv,
    DpiMismatch,
    DpiZero,

    // --- Single-pixel buffer ---
    SpFill(u8, u8, u8, u8),
    SpSub(u8, u8, u8, u8),
    SpNoViewport,

    // --- Tearing (wp_tearing_control_v1) ---
    /// Publish `set_presentation_hint(async|vsync)` on the main surface. In the
    /// compositor's `Exclusive` mode `async` tags this client as a pacer.
    Tearing(bool),
    /// Self-drive commits at N Hz on the subject's OWN timer, ignoring frame
    /// callbacks — a game in IMMEDIATE present mode. `0` restores the default
    /// animation-only tick. This is the independent variable the pacing test
    /// sweeps; driving off callbacks instead would make the measurement circular.
    CommitRate(u32),
    /// Draw the live FRAME/COMMIT counter into the subject's overlay. Off gives
    /// pixel-identical frames — the control case for judging whether a seen tear
    /// is real.
    ShowCounter(bool),

    // --- Lifecycle ---
    Map,
    Unmap,
    MapCycle(bool),
    Size(u32, u32),
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
            Command::DecoMode(m) => {
                let _ = write!(
                    s,
                    "deco-mode {}",
                    match m {
                        DecoMode::Server => "server",
                        DecoMode::Client => "client",
                        DecoMode::None => "none",
                    }
                );
            }
            Command::DecoIgnore => s.push_str("deco-ignore"),
            Command::DecoBadsize => s.push_str("deco-badsize"),

            Command::BufAgreed => s.push_str("buf-agreed"),
            Command::BufDelta(d) => {
                let _ = write!(s, "buf-delta {d}");
            }
            Command::BufZero => s.push_str("buf-zero"),
            Command::BufNoack => s.push_str("buf-noack"),
            Command::BufPreack => s.push_str("buf-preack"),
            Command::GeoMismatch => s.push_str("geo-mismatch"),

            Command::PopupAdd => s.push_str("popup-add"),
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
            Command::PopupNest => s.push_str("popup-nest"),
            Command::PopupClose => s.push_str("popup-close"),

            Command::SubAdd => s.push_str("sub-add"),
            Command::SubMove(x, y) => {
                let _ = write!(s, "sub-move {x} {y}");
            }
            Command::SubSync => s.push_str("sub-sync"),
            Command::SubDesync => s.push_str("sub-desync"),
            Command::SubNest => s.push_str("sub-nest"),
            Command::SubRemove => s.push_str("sub-remove"),

            Command::VpDest(w, h) => {
                let _ = write!(s, "vp-dest {w} {h}");
            }
            Command::VpDestDelta(d) => {
                let _ = write!(s, "vp-dest-delta {d}");
            }
            Command::VpSrc(x, y, w, h) => {
                let _ = write!(s, "vp-src {x} {y} {w} {h}");
            }
            Command::VpAnimate(on) => {
                let _ = write!(s, "vp-animate {}", on8(*on));
            }
            Command::VpUnset => s.push_str("vp-unset"),
            Command::VpBad => s.push_str("vp-bad"),

            Command::FsHonor => s.push_str("fs-honor"),
            Command::FsIgnore => s.push_str("fs-ignore"),
            Command::FsScale(n) => {
                let _ = write!(s, "fs-scale {n}");
            }
            Command::FsNoViewport => s.push_str("fs-noviewport"),
            Command::FsMismatch => s.push_str("fs-mismatch"),

            Command::DpiHonor => s.push_str("dpi-honor"),
            Command::DpiIgnore => s.push_str("dpi-ignore"),
            Command::DpiScale(n) => {
                let _ = write!(s, "dpi-scale {n}");
            }
            Command::DpiNondiv => s.push_str("dpi-nondiv"),
            Command::DpiMismatch => s.push_str("dpi-mismatch"),
            Command::DpiZero => s.push_str("dpi-zero"),

            Command::SpFill(r, g, b, a) => {
                let _ = write!(s, "sp-fill {r} {g} {b} {a}");
            }
            Command::SpSub(r, g, b, a) => {
                let _ = write!(s, "sp-sub {r} {g} {b} {a}");
            }
            Command::SpNoViewport => s.push_str("sp-noviewport"),

            Command::Tearing(on) => {
                let _ = write!(s, "tearing {}", on8(*on));
            }
            Command::CommitRate(hz) => {
                let _ = write!(s, "commitrate {hz}");
            }
            Command::ShowCounter(on) => {
                let _ = write!(s, "showcounter {}", on8(*on));
            }
            Command::Map => s.push_str("map"),
            Command::Unmap => s.push_str("unmap"),
            Command::MapCycle(on) => {
                let _ = write!(s, "mapcycle {}", on8(*on));
            }
            Command::Size(w, h) => {
                let _ = write!(s, "size {w} {h}");
            }
            Command::Quit => s.push_str("quit"),
        }
        s
    }

    /// Parse a single wire line, ignoring surrounding whitespace. Returns `None` for an
    /// unknown verb or malformed args.
    pub fn parse(line: &str) -> Option<Command> {
        let mut it = line.split_whitespace();
        let verb = it.next()?;
        // Small helpers for positional args.
        let mut next = || it.next();
        Some(match verb {
            "deco-mode" => Command::DecoMode(match next()? {
                "server" => DecoMode::Server,
                "client" => DecoMode::Client,
                "none" => DecoMode::None,
                _ => return None,
            }),
            "deco-ignore" => Command::DecoIgnore,
            "deco-badsize" => Command::DecoBadsize,

            "buf-agreed" => Command::BufAgreed,
            "buf-delta" => Command::BufDelta(next()?.parse().ok()?),
            "buf-zero" => Command::BufZero,
            "buf-noack" => Command::BufNoack,
            "buf-preack" => Command::BufPreack,
            "geo-mismatch" => Command::GeoMismatch,

            "popup-add" => Command::PopupAdd,
            "popup-anchor" => Command::PopupAnchor(Anchor::parse(next()?)?),
            "popup-gravity" => Command::PopupGravity(Anchor::parse(next()?)?),
            "popup-off" => Command::PopupOff(next()?.parse().ok()?, next()?.parse().ok()?),
            "popup-size" => Command::PopupSize(next()?.parse().ok()?, next()?.parse().ok()?),
            "popup-move" => Command::PopupMove(next()?.parse().ok()?, next()?.parse().ok()?),
            "popup-nest" => Command::PopupNest,
            "popup-close" => Command::PopupClose,

            "sub-add" => Command::SubAdd,
            "sub-move" => Command::SubMove(next()?.parse().ok()?, next()?.parse().ok()?),
            "sub-sync" => Command::SubSync,
            "sub-desync" => Command::SubDesync,
            "sub-nest" => Command::SubNest,
            "sub-remove" => Command::SubRemove,

            "vp-dest" => Command::VpDest(next()?.parse().ok()?, next()?.parse().ok()?),
            "vp-dest-delta" => Command::VpDestDelta(next()?.parse().ok()?),
            "vp-src" => Command::VpSrc(
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
            ),
            "vp-animate" => Command::VpAnimate(parse_on(next()?)?),
            "vp-unset" => Command::VpUnset,
            "vp-bad" => Command::VpBad,

            "fs-honor" => Command::FsHonor,
            "fs-ignore" => Command::FsIgnore,
            "fs-scale" => Command::FsScale(next()?.parse().ok()?),
            "fs-noviewport" => Command::FsNoViewport,
            "fs-mismatch" => Command::FsMismatch,

            "dpi-honor" => Command::DpiHonor,
            "dpi-ignore" => Command::DpiIgnore,
            "dpi-scale" => Command::DpiScale(next()?.parse().ok()?),
            "dpi-nondiv" => Command::DpiNondiv,
            "dpi-mismatch" => Command::DpiMismatch,
            "dpi-zero" => Command::DpiZero,

            "sp-fill" => Command::SpFill(
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
            ),
            "sp-sub" => Command::SpSub(
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
                next()?.parse().ok()?,
            ),
            "sp-noviewport" => Command::SpNoViewport,

            "tearing" => Command::Tearing(parse_on(next()?)?),
            "commitrate" => Command::CommitRate(next()?.parse().ok()?),
            "showcounter" => Command::ShowCounter(parse_on(next()?)?),
            "map" => Command::Map,
            "unmap" => Command::Unmap,
            "mapcycle" => Command::MapCycle(parse_on(next()?)?),
            "size" => Command::Size(next()?.parse().ok()?, next()?.parse().ok()?),
            "quit" => Command::Quit,

            _ => return None,
        })
    }
}

fn on8(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
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
            Command::DecoMode(DecoMode::Server),
            Command::DecoIgnore,
            Command::BufDelta(-40),
            Command::PopupOff(400, 0),
            Command::PopupAnchor(Anchor::BottomRight),
            Command::VpSrc(0.0, 0.0, 128.5, 64.25),
            Command::VpAnimate(true),
            Command::FsScale(180),
            Command::DpiScale(2),
            Command::SpFill(255, 0, 0, 255),
            Command::Size(800, 600),
            Command::Quit,
        ];
        for c in cases {
            let line = c.encode();
            assert_eq!(Command::parse(&line), Some(c.clone()), "line = {line:?}");
        }
    }
}
