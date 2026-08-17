//! One in-progress read of a clipboard flavor out of the client that owns it.

use std::os::fd::OwnedFd;
use std::sync::Arc;


/// An in-progress read of one flavor.
///
/// Owned by the capture worker while the copying client is alive, and handed back to the
/// compositor thread at that client's exit so `selection_source_destroyed` can finish the
/// read in place — that hook must answer synchronously and cannot wait on the worker.
pub struct Reader {
    pub generation: u64,
    pub mime: String,
    /// Position of this flavor in the client's advertised list. Capture order is ours;
    /// the offer is rebuilt in the client's, so this rides along untouched.
    pub order: usize,
    pub fd: Arc<OwnedFd>,
    /// Bytes to attempt per `read`: this pipe's actual capacity, so one syscall drains a
    /// full pipe and no pass ever makes room for more than the pipe can hold.
    pub chunk: usize,
    pub buffer: Vec<u8>,
    /// Set once the reader has hit EOF, been abandoned, or been superseded by a newer
    /// copy. The worker drops it on its next pass.
    pub finished: bool,
    /// `buffer.len()` as of the last idle check. A reader that has not grown since is
    /// making no progress and gets abandoned — that is what keeps a client advertising a
    /// mime type it cannot produce from holding a read open forever, without cutting off
    /// a large payload that is still arriving.
    pub last_len: usize,
}

/// Outcome of one non-blocking pump of a reader's fd.
pub enum Pump {
    /// The writer is gone and everything it wrote has been consumed.
    Eof,
    /// The pipe is empty but a writer still holds the other end.
    Blocked,
    /// Read error, or the flavor outgrew the budget. Drop it.
    Abandon,
}

/// Drain everything currently readable from `reader`'s pipe into its buffer.
///
/// The read end is non-blocking, which is what lets this serve two very different
/// callers: the capture worker while the client is alive, and
/// `selection_source_destroyed` finishing the read in place after it has died. In the
/// latter case the client's exit has already closed the write end, so the loop consumes
/// the buffered bytes and reaches [`Pump::Eof`] without ever blocking.
///
/// [`Pump::Blocked`] from the destroy path means some other process still holds the
/// write end (a forked child, say) — do not spin on it, drop the flavor.
pub fn pump(reader: &mut Reader, budget: usize) -> Pump {
    loop {
        // Read into the tail of the buffer rather than a scratch chunk. At this size a
        // per-call scratch would cost an allocation and a copy on every pass, whereas the
        // buffer's capacity is reused across them.
        let start = reader.buffer.len();
        reader.buffer.resize(start + reader.chunk, 0);
        let result = smithay::reexports::rustix::io::read(&*reader.fd, &mut reader.buffer[start..]);
        // `resize` only made room — keep what actually arrived and nothing more.
        reader.buffer.truncate(start + result.unwrap_or(0));
        match result {
            Ok(0) => return Pump::Eof,
            Ok(len) if start + len > budget => return Pump::Abandon,
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Pump::Blocked,
            Err(_) => return Pump::Abandon,
        }
    }
}
