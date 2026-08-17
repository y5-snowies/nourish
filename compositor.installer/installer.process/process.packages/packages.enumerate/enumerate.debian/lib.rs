//! Debian / Ubuntu (`apt`) runtime package groups — the same soname set as the Fedora
//! table (`enumerate.fedora`) expressed in apt names. Pure std.
//!
//! Two cross-release hazards are handled here rather than left to fail:
//!   * ffmpeg: the libav* runtime libs are soversion-suffixed and differ per release
//!     (bookworm libavcodec59, noble …60, trixie …61). We install the `ffmpeg` package
//!     instead — it depends on exactly the matching libav* runtime, so the right
//!     soversion is pulled without us naming it. Release-independent.
//!   * libdisplay-info: soversion-suffixed with NO metapackage, so it IS named per
//!     release — `libdisplay-info2` on bookworm(-backports)/trixie, `libdisplay-info1`
//!     on Ubuntu noble. (bookworm carries it only in bookworm-backports, so the apt path
//!     enables that suite on release 12 — see execute.packages / enumerate.install.)
//!
//! GTK is deliberately NOT named: the `64-bit time_t` transition renamed it to
//! `libgtk-3-0t64` on trixie + noble but left it `libgtk-3-0` on bookworm. Installing
//! `libwebkit2gtk-4.1-0` pulls the correct GTK for the release as a dependency, so the
//! devtool group sidesteps the t64 rename entirely.

pub mod debian;
pub use debian::*;
