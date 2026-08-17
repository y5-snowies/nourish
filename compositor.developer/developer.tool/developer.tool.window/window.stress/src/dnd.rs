//! Extra toplevels, their teardown, and the drag-and-drop payload plumbing.
//!
//! Split out of `subject.rs` because none of it needs the `Subject` type: the
//! window graph is plain proxies and the payload transfer is a pipe. The
//! `Dispatch` impls have to live next to `Subject`, so they stay in the binary.

use std::io::{Read, Write};
use std::os::fd::OwnedFd;

use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::xdg::shell::client::{xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel};

/// The single type we offer and accept. `text/plain;charset=utf-8` is what a
/// real text drag carries, so a drop onto a foreign app is a valid test too.
pub const MIME: &str = "text/plain;charset=utf-8";

// ---- Tab strip -------------------------------------------------------------
//
// Every window draws a row of tabs. Pressing one and dragging tears it into its
// own window via `xdg_toplevel_drag_v1`; dropping it on another window's strip
// docks it there. This is the shape a browser actually uses, and unlike the
// `drag` arm commands it needs no mode: pressing a tab IS the intent.

pub const TAB_W: i32 = 74;
pub const TAB_H: i32 = 18;
/// Top of the strip, below the window's title text.
pub const TAB_Y: i32 = 40;
pub const TAB_X0: i32 = 6;

/// Which window a tab came from / went to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Main,
    /// Extra window, by its stable `Win::id`.
    Win(u32),
}

/// A tab lifted out of a strip, remembered so a cancelled drag can put it back.
#[derive(Clone, Debug)]
pub struct TornTab {
    pub label: String,
    pub origin: Origin,
    /// Position it occupied, so a restore does not reorder the strip.
    pub index: usize,
}

/// Which tab (if any) a surface-local click lands on.
pub fn tab_at(x: f64, y: f64, count: usize) -> Option<usize> {
    if y < TAB_Y as f64 || y >= (TAB_Y + TAB_H) as f64 || x < TAB_X0 as f64 {
        return None;
    }
    let i = ((x - TAB_X0 as f64) / TAB_W as f64) as usize;
    (i < count).then_some(i)
}

/// One additional toplevel's complete object graph.
///
/// Exists so teardown can be exercised for real: `unmap` attaches a nil buffer
/// and keeps every object, which never reaches the compositor's destruction
/// paths. Destroying the graph does.
pub struct Win {
    /// Stable id carried in the `xdg_surface` userdata, so a configure can find
    /// its window even after earlier ones were closed and the Vec reindexed.
    pub id: u32,
    pub surface: WlSurface,
    pub xdg: XdgSurface,
    pub toplevel: XdgToplevel,
    /// Set once the first `xdg_surface.configure` has been acked and a buffer
    /// attached — `attach` on an unmapped toplevel is legal for toplevel-drag,
    /// so the distinction matters to the drag commands.
    pub mapped: bool,
    pub configured: bool,
    pub size: (i32, i32),
    /// This window's tab strip.
    pub tabs: Vec<String>,
}

impl Win {
    /// Destroy in protocol order: children before parents. Getting this
    /// backwards is itself a protocol error, so the order is the test.
    pub fn destroy(self) {
        self.toplevel.destroy();
        self.xdg.destroy();
        self.surface.destroy();
    }
}

/// Serve a `wl_data_source.send`: write the payload and close.
///
/// The fd must be dropped, not just written to — the receiving client reads
/// until EOF, so leaking it hangs the peer rather than failing visibly.
pub fn write_payload(fd: OwnedFd, text: &str) {
    let mut f = std::fs::File::from(fd);
    if let Err(e) = f.write_all(text.as_bytes()) {
        eprintln!("[subject] dnd: writing payload failed: {e}");
    }
}

/// Put the transfer pipe into non-blocking mode.
///
/// Mandatory, not an optimisation. For a drag that starts and ends in this
/// process the writer is our own `wl_data_source.send` handler, so a blocking
/// read freezes the event loop that would have delivered the event that does
/// the writing — the client wedges with the drag half-finished and stops
/// answering anything, including close requests. A cancelled drop is the same
/// story with no writer at all.
pub fn set_nonblocking(fd: &OwnedFd) {
    use std::os::fd::AsRawFd;
    unsafe {
        let f = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        if f < 0 || libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, f | libc::O_NONBLOCK) < 0 {
            eprintln!("[subject] dnd: could not set O_NONBLOCK on the transfer pipe");
        }
    }
}

/// One non-blocking read step.
///
/// `Ok(true)` = EOF, transfer complete. `Ok(false)` = nothing yet, pump the
/// queue and come back.
pub fn read_step(f: &mut std::fs::File, out: &mut Vec<u8>) -> std::io::Result<bool> {
    let mut buf = [0u8; 4096];
    loop {
        match f.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}
