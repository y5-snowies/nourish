//! What the clipboard persistence layer captures, in what order, and how much of it.
//!
//! Capturing is eager — a dead client cannot be read, so the bytes must be pulled while
//! the source still exists — which makes these limits the difference between "the
//! clipboard survives closing the app" and "one copy pins hundreds of megabytes".

use std::time::Duration;

/// Bytes held for one selection: the total across all flavors, and equally the ceiling
/// for any single flavor.
pub const BUDGET: usize = 128 * 1024 * 1024;

/// A read that makes no progress for this long is abandoned.
///
/// IDLE, not total duration. A large payload that is still arriving must not be cut off
/// mid-transfer, but a client that advertises a mime type it cannot actually produce
/// must not hang the read forever.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(4);

/// The whole capture, start to finish.
///
/// [`IDLE_TIMEOUT`] alone cannot bound it: it is re-armed for as long as ANY flavor is
/// still growing, so a client dribbling one byte per window holds its readers — and up
/// to [`BUDGET`] of buffer — indefinitely. This is the ceiling that makes "one client
/// cannot leak inside us" true.
///
/// Generous on purpose. It is a leak bound, not a performance target, and a slow but
/// honest producer must not be punished for being slow. Flavors that completed before it
/// fires are kept; the ones still in flight are dropped, because a truncated flavor is
/// worse than an absent one — the receiver cannot tell the difference.
pub const TOTAL_CAPTURE: Duration = Duration::from_secs(60);

/// A paste that makes no progress for this long is abandoned.
///
/// `wl_data_offer.receive` has the client hand us the write end of its own pipe, so a
/// client that asks for a paste and then stops reading holds that fd — and the payload it
/// points at — until something takes it back. This is the usual thing that does.
///
/// IDLE for the same reason as [`IDLE_TIMEOUT`]: "stopped consuming" and "consuming
/// slowly" must not be the same verdict, and only the first deserves to lose its data.
pub const PASTE_IDLE: Duration = Duration::from_secs(4);

/// The whole paste, start to finish — the ceiling [`PASTE_IDLE`] cannot provide, since a
/// client reading one byte per window would keep pushing its idle deadline forward.
///
/// Expiry closes the fd, which the client reads as EOF part-way through. It cannot tell
/// that from a complete transfer (`receive` carries no length), so this is deliberately
/// generous: at [`BUDGET`] it is several times what any local client needs, and anything
/// slower than that is not a paste anyone is waiting on.
pub const TOTAL_PASTE: Duration = Duration::from_secs(60);

/// Capacity to ASK for on the capture pipe. What we get is whatever the machine allows.
///
/// This pipe is ours — `arm` creates it and hands the client the write end — so enlarging
/// it is ours to do. The client's end is deliberately blocking, so the pipe's capacity is
/// how far it gets before it has to wait on us.
///
/// 1 MiB because that is the kernel's default `/proc/sys/fs/pipe-max-size`, and a request
/// above the limit is DENIED rather than clamped — overshooting leaves the default, so it
/// yields less, not more. The limit is a per-machine sysctl though, and the request can be
/// refused far below it (the per-user pipe page budget counts, and some sandboxes block the
/// call entirely), so `arm` halves this until the kernel agrees rather than trusting it.
pub const CAPTURE_PIPE: usize = 1024 * 1024;

/// Read size when the pipe's real capacity cannot be queried.
///
/// Normally the capture reads exactly one pipe-capacity at a time — see `Reader::chunk`,
/// which is filled in from `F_GETPIPE_SZ`. Reading more than the pipe can hold is dead
/// weight, and reading less just costs extra syscalls, so the true capacity is the only
/// right answer and this is only the fallback.
pub const READ_CHUNK: usize = 64 * 1024;

/// One `write()` worth of bytes when serving a paste.
///
/// A quarter of what we ask for on the capture side: we know what a capture chunk costs us
/// (nothing but an append), and we know nothing about what a client does with a paste chunk.
///
/// This bounds OUR syscall per pass, so one large paste cannot starve the other transfers
/// sharing it. It does NOT bound what the client receives per `read`: we refill as soon as
/// `poll` reports space, so its pipe converges to ITS capacity whatever we do here, and that
/// capacity is the client's choice — we do not resize a pipe we did not create. In practice
/// `EAGAIN` at the client's capacity binds first and this ceiling rarely applies at all.
pub const WRITE_CHUNK: usize = CAPTURE_PIPE / 4;

/// Text-ish flavors. Small and quick to transfer, so they are captured first.
fn is_text(mime: &str) -> bool {
    mime.starts_with("text/") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

/// Order the client's advertised mime types for capture: text first, then the client's
/// own order.
///
/// We do not second-guess the client with a size heuristic. Its ordering expresses which
/// flavor IT considers primary, and the larger encoding is frequently the more faithful
/// one. Text is the single exception — it is cheap and arrives fast, so taking it first
/// means a big image cannot spend the whole budget before the text of the same copy is
/// safe.
///
/// The index that rides along is each flavor's position in the client's list. The offer
/// built after the client exits is sorted back into that order, so a receiver that picks
/// the first acceptable type negotiates the same flavor before and after the exit.
pub fn ordered(mime_types: &[String]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = mime_types.iter().cloned().enumerate().collect();
    // Stable, so the client's relative order survives within each bucket.
    out.sort_by_key(|(_, mime)| u8::from(!is_text(mime)));
    out
}
