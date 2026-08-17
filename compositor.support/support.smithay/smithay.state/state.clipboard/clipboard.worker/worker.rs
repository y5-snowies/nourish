//! The clipboard transfer worker: one long-lived thread that owns every in-flight pipe,
//! in both directions — reads that capture a copy, writes that serve a paste.
//!
//! Both used to run on the compositor thread from calloop sources, and that is what made a
//! large transfer a stall. The loops run until the pipe is empty (or full), and a client on
//! another core refills (or drains) it in between, so a single callback could move the whole
//! budget inline. Capping bytes per callback does not fix it — a level-triggered source that
//! is still ready re-fires immediately, so the total main-thread cost is unchanged. Neither
//! can a timer: calloop is single-threaded, so nothing fires while the callback is running.
//! Moving the loop off the thread is the only thing that actually bounds it.
//!
//! The thread does not poll on an interval. It blocks in `poll()` over its live pipes plus a
//! wakeup pipe the compositor pokes, and the poll timeout is the time until the nearest
//! deadline — so every wake is either something to move, something to answer, or something
//! that has just come due, and an idle worker blocks indefinitely at no cost.
//!
//! Nothing is shared. Fds move in over [`Command`], finished flavors move out over [`Done`],
//! and a calloop `Ping` wakes the compositor so its drain collects them. There is no lock,
//! and therefore no worker state the compositor can observe half-written.
//!
//! Nothing here may log clipboard CONTENT — mime names, byte counts and generations only.

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::time::{Duration, Instant};

use compositor_support_smithay_state_clipboard_policy::policy;
use compositor_support_smithay_state_clipboard_pump::pump::{self, Pump, Reader};
use smithay::reexports::calloop::ping::Ping;
use smithay::reexports::rustix;
use smithay::reexports::rustix::event::{PollFd, PollFlags, Timespec};
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::backend::ClientId;

/// How long the compositor will wait for the worker to hand its reads back.
///
/// A safety valve, not a design parameter: a command pokes the wakeup pipe, so `poll`
/// returns at once and the answer takes microseconds. If it somehow does not, the destroy
/// path proceeds with whatever already completed rather than stalling the compositor inside
/// a wayland destructor.
const RECLAIM_GRACE: Duration = Duration::from_millis(100);

/// A flavor the worker read to EOF.
pub struct Done {
    pub generation: u64,
    pub order: usize,
    pub mime: String,
    pub bytes: Vec<u8>,
}

enum Command {
    /// A new copy: drop everything older, reset the budget, restart the clocks.
    Arm { generation: u64 },
    /// Read one flavor of the current head.
    Read { generation: u64, order: usize, mime: String, fd: OwnedFd, chunk: usize },
    /// Write a held flavor back to a pasting client.
    Write { client: ClientId, mime: String, fd: OwnedFd, bytes: Arc<Vec<u8>> },
    /// Hand the still-unfinished reads back — see [`Worker::reclaim`].
    Reclaim { reply: SyncSender<Vec<Reader>> },
}

/// One paste in progress.
struct Writer {
    /// Who asked. Pastes are judged per client, never per pipe — see [`Transfers::push`].
    client: ClientId,
    mime: String,
    fd: OwnedFd,
    bytes: Arc<Vec<u8>>,
    offset: usize,
    /// `receive` hands us the write end of the CLIENT's pipe, so a client that asks for a
    /// paste and then stops reading holds that fd and pins the payload. These are the
    /// bounds: pushed forward on every byte that lands, so a slow-but-honest consumer is
    /// never mistaken for one that walked away.
    idle_deadline: Instant,
    /// The ceiling the idle window cannot provide — a client reading one byte per window
    /// would otherwise renew itself forever.
    deadline: Instant,
    done: bool,
}

/// The compositor's end of the worker.
pub struct Worker {
    commands: Sender<Command>,
    /// Write end of the wakeup pipe: a byte here is what makes the worker's `poll` return
    /// so it looks at the command channel.
    wake: OwnedFd,
    done: Receiver<Done>,
}

impl Worker {
    /// Spawn the worker. `ping` is what wakes the compositor once a flavor is ready.
    pub fn start(ping: Ping) -> Option<Worker> {
        let pipe = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC);
        let Ok((wake_read, wake_write)) = pipe else {
            warn!("clipboard worker: could not create the wakeup pipe");
            return None;
        };
        // Both ends non-blocking: the worker must never block draining wakeups, and the
        // compositor must never block poking a wakeup pipe that is already full.
        let read_flags = rustix::fs::fcntl_setfl(&wake_read, OFlags::RDONLY | OFlags::NONBLOCK);
        let write_flags = rustix::fs::fcntl_setfl(&wake_write, OFlags::WRONLY | OFlags::NONBLOCK);
        if read_flags.is_err() || write_flags.is_err() {
            warn!("clipboard worker: O_NONBLOCK failed on the wakeup pipe");
            return None;
        }
        let (commands, inbox) = channel();
        let (finished, done) = channel();
        let spawned = std::thread::Builder::new()
            .name("y5-clipboard".to_string())
            .spawn(move || run(inbox, wake_read, finished, ping));
        if let Err(err) = spawned {
            warn!("clipboard worker: could not spawn err={err:?}");
            return None;
        }
        Some(Worker { commands, wake: wake_write, done })
    }

    /// Queue a command and wake the worker to see it.
    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            return;
        }
        // One byte is enough to end a `poll`; a pipe that is already full carries the same
        // meaning, so a refused write needs no handling.
        let _ = rustix::io::write(&self.wake, &[0u8]);
    }

    /// Invalidate whatever is in flight and start a new head. Idempotent, so both the
    /// `new_selection` hook (which must cancel even when the clipboard was merely cleared)
    /// and the drain (which may be the first to start the worker at all) can send it.
    pub fn arm(&self, generation: u64) {
        self.send(Command::Arm { generation });
    }

    /// `chunk` is the pipe's real capacity — see `widen` in the wire crate.
    pub fn read(&self, generation: u64, order: usize, mime: String, fd: OwnedFd, chunk: usize) {
        self.send(Command::Read { generation, order, mime, fd, chunk });
    }

    /// Serve a paste. The payload is shared, not copied — several clients may be reading
    /// the same flavor, and a single one may ask for it repeatedly.
    ///
    /// Not generation-gated: the compositor checks that the flavor is still current when it
    /// looks the bytes up, and a paste already in flight belongs to whoever asked for it,
    /// not to whatever the clipboard holds by the time it finishes.
    pub fn write(&self, client: ClientId, mime: String, fd: OwnedFd, bytes: Arc<Vec<u8>>) {
        self.send(Command::Write { client, mime, fd, bytes });
    }

    /// Everything the worker has finished since the last call.
    pub fn collect(&self) -> Vec<Done> {
        self.done.try_iter().collect()
    }

    /// Take back the reads that are still in flight.
    ///
    /// Used at client exit: the client's own exit closed every write end, so those pipes
    /// now drain straight to EOF and the destroy path can finish them synchronously without
    /// blocking. That is what keeps a copy-then-immediately-close from losing a transfer
    /// that was still moving.
    pub fn reclaim(&self) -> Vec<Reader> {
        let (reply, answer) = sync_channel(1);
        self.send(Command::Reclaim { reply });
        answer.recv_timeout(RECLAIM_GRACE).unwrap_or_else(|_| {
            warn!("clipboard worker: reclaim timed out; offering only completed flavors");
            Vec::new()
        })
    }
}

/// Everything the worker owns. Never leaves the worker thread.
struct Transfers {
    readers: Vec<Reader>,
    writers: Vec<Writer>,
    generation: u64,
    /// Bytes already handed to the compositor for this head — the other half of the budget,
    /// which [`Transfers::headroom`] charges against.
    sent: usize,
    deadline: Instant,
    idle_check: Instant,
}

impl Transfers {
    fn apply(&mut self, command: Command) {
        match command {
            Command::Arm { generation } => {
                // Dropping the readers closes our read ends, which also releases a client
                // still blocked writing into a pipe of the copy it just replaced. Writers
                // are left alone: a paste belongs to the client that asked for it, not to
                // whatever the clipboard happens to hold now.
                self.readers.clear();
                self.generation = generation;
                self.sent = 0;
                let now = Instant::now();
                self.deadline = now + policy::TOTAL_CAPTURE;
                self.idle_check = now + policy::IDLE_TIMEOUT;
            }
            Command::Read { generation, order, mime, fd, chunk } => {
                if generation != self.generation {
                    return;
                }
                self.readers.push(Reader {
                    generation,
                    mime,
                    order,
                    fd: Arc::new(fd),
                    chunk,
                    buffer: Vec::new(),
                    finished: false,
                    last_len: 0,
                });
            }
            Command::Write { client, mime, fd, bytes } => {
                self.writers.push(Writer {
                    client,
                    mime,
                    fd,
                    bytes,
                    offset: 0,
                    idle_deadline: Instant::now() + policy::PASTE_IDLE,
                    deadline: Instant::now() + policy::TOTAL_PASTE,
                    done: false,
                });
            }
            Command::Reclaim { reply } => {
                let live = std::mem::take(&mut self.readers);
                let _ = reply.send(live.into_iter().filter(|reader| !reader.finished).collect());
            }
        }
    }

    /// When the next deadline comes due, which is exactly how long `poll` may wait.
    ///
    /// `None` means nothing is in flight, so nothing can come due and the only thing that
    /// can wake us is a command — `poll` then blocks indefinitely at no cost.
    fn next_deadline(&self) -> Option<Duration> {
        let mut soonest = match self.readers.is_empty() {
            true => None,
            false => Some(self.idle_check.min(self.deadline)),
        };
        for writer in &self.writers {
            let due = writer.deadline.min(writer.idle_deadline);
            soonest = Some(soonest.map_or(due, |at| at.min(due)));
        }
        soonest.map(|at| at.saturating_duration_since(Instant::now()))
    }

    /// Block until a pipe is ready, a command arrives, or the nearest deadline comes due.
    fn wait(&self, wake: &OwnedFd) {
        let mut fds = Vec::with_capacity(self.readers.len() + self.writers.len() + 1);
        fds.push(PollFd::new(wake, PollFlags::IN));
        for reader in &self.readers {
            fds.push(PollFd::new(&*reader.fd, PollFlags::IN));
        }
        for writer in &self.writers {
            fds.push(PollFd::new(&writer.fd, PollFlags::OUT));
        }
        let timeout = self.next_deadline().map(|left| Timespec {
            tv_sec: left.as_secs() as _,
            tv_nsec: left.subsec_nanos() as _,
        });
        // EINTR and friends just mean "look again". The passes below tolerate a spurious
        // wake, and readiness that arrives late is picked up on the next deadline.
        let _ = rustix::event::poll(&mut fds, timeout.as_ref());
    }

    /// What the reader at `index` may still buffer: the budget, less what has already been
    /// handed over, less what the OTHER live readers hold. Charging every reader the whole
    /// budget instead would make peak memory the budget times the flavor count.
    fn headroom(&self, index: usize) -> usize {
        let others: usize = self
            .readers
            .iter()
            .enumerate()
            .filter(|(at, reader)| *at != index && !reader.finished)
            .map(|(_, reader)| reader.buffer.len())
            .sum();
        policy::BUDGET.saturating_sub(self.sent + others)
    }

    /// Drop the capture when the client stops producing, and cap it as a whole.
    ///
    /// Progress is measured per CAPTURE, not per flavor. Every flavor is requested at once,
    /// so a single-threaded client serves them in sequence — and because its end of the pipe
    /// is deliberately blocking, the second flavor receives nothing until the first has been
    /// drained in full. Judging each reader on its own would therefore abandon flavors that
    /// are simply waiting their turn, and a large first flavor would cost the client every
    /// other one. If ANY reader grew, the client is alive and working.
    ///
    /// So the idle window only fires when the client has gone quiet across the board, and
    /// then it ends the whole capture rather than one flavor of it. [`policy::TOTAL_CAPTURE`]
    /// is the separate ceiling, for a client that dribbles one byte per window forever.
    ///
    /// Flavors that already reached EOF were admitted then and are unaffected. Partial
    /// buffers are dropped rather than handed over — a truncated flavor is worse than an
    /// absent one, because the receiver cannot tell.
    fn expire(&mut self) {
        let now = Instant::now();
        let expired = now >= self.deadline;
        if !expired && now < self.idle_check {
            return;
        }
        let live = || self.readers.iter().filter(|reader| !reader.finished);
        let quiet = !live().any(|reader| reader.buffer.len() != reader.last_len);
        if !expired && !quiet {
            for reader in self.readers.iter_mut().filter(|reader| !reader.finished) {
                reader.last_len = reader.buffer.len();
            }
            self.idle_check = now + policy::IDLE_TIMEOUT;
            return;
        }
        for reader in self.readers.iter_mut().filter(|reader| !reader.finished) {
            match expired {
                true => warn!("clipboard capture hit the total deadline mime={}", reader.mime),
                false => warn!("clipboard capture went quiet mime={}", reader.mime),
            }
            reader.finished = true;
        }
    }

    /// One non-blocking pass over every capture pipe.
    fn pull(&mut self, finished: &Sender<Done>, ping: &Ping) {
        for index in 0..self.readers.len() {
            if self.readers[index].finished {
                continue;
            }
            let headroom = self.headroom(index);
            match pump::pump(&mut self.readers[index], headroom) {
                Pump::Blocked => {}
                Pump::Eof => {
                    let reader = &mut self.readers[index];
                    reader.finished = true;
                    let done = Done {
                        generation: reader.generation,
                        order: reader.order,
                        mime: reader.mime.clone(),
                        bytes: std::mem::take(&mut reader.buffer),
                    };
                    self.sent += done.bytes.len();
                    trace!("clipboard worker read mime={} bytes={}", done.mime, done.bytes.len());
                    if finished.send(done).is_ok() {
                        ping.ping();
                    }
                }
                Pump::Abandon => {
                    let reader = &mut self.readers[index];
                    reader.finished = true;
                    warn!(
                        "clipboard worker abandoned mime={} (read error or over budget)",
                        reader.mime
                    );
                }
            }
        }
        self.expire();
        self.readers.retain(|reader| !reader.finished);
    }

    /// One non-blocking pass over every paste in flight.
    ///
    /// A chunk per writer per pass rather than the whole payload: this thread is not the
    /// compositor's, so a long write costs it nothing, but one large paste must not starve
    /// the other transfers sharing the pass.
    fn push(&mut self) {
        let now = Instant::now();
        // Clients that consumed something this pass. Progress is judged per CLIENT, not per
        // pipe: a client may `receive` several mime types at once and drain them one after
        // another, which leaves the pipes it has not reached yet sitting full and looking
        // exactly like a paste it walked away from. Any pipe of a client moving proves it is
        // still consuming, so none of its pastes has stalled. Writers of OTHER clients are
        // unrelated and never vouch for each other.
        let mut consuming: Vec<ClientId> = Vec::new();
        for writer in self.writers.iter_mut() {
            let before = writer.offset;
            let ceiling = (writer.offset + policy::WRITE_CHUNK).min(writer.bytes.len());
            loop {
                if writer.offset >= writer.bytes.len() {
                    writer.done = true;
                    break;
                }
                if writer.offset >= ceiling {
                    break;
                }
                match rustix::io::write(&writer.fd, &writer.bytes[writer.offset..ceiling]) {
                    Ok(0) => {
                        writer.done = true;
                        break;
                    }
                    Ok(len) => writer.offset += len,
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                    // Usually EPIPE: the pasting client lost interest, crashed or exited.
                    Err(_) => {
                        writer.done = true;
                        break;
                    }
                }
            }
            if writer.offset != before && !consuming.contains(&writer.client) {
                consuming.push(writer.client.clone());
            }
        }
        // Judged only after the whole pass has run, so a pipe that just moved is never
        // expired for the window it was moving in, and a sibling's progress has already
        // been recorded by the time this reads it.
        for writer in self.writers.iter_mut() {
            if consuming.contains(&writer.client) {
                writer.idle_deadline = now + policy::PASTE_IDLE;
            }
            if writer.done {
                continue;
            }
            // Expiry closes the fd, so the client sees EOF part-way through and cannot tell
            // that from a complete transfer. Accepted: the alternative is letting a client
            // that stopped reading pin the payload and the fd for as long as it likes.
            if now >= writer.deadline {
                writer.done = true;
                warn!("clipboard paste hit the total deadline mime={}", writer.mime);
            } else if now >= writer.idle_deadline {
                writer.done = true;
                warn!("clipboard paste stalled mime={} (client stopped reading)", writer.mime);
            }
        }
        // Dropping a writer closes its fd, which is what gives the client its EOF.
        self.writers.retain(|writer| !writer.done);
    }
}

/// Consume the wakeup bytes so the next `poll` blocks again.
fn clear(wake: &OwnedFd) {
    let mut sink = [0u8; 64];
    while matches!(rustix::io::read(wake, &mut sink), Ok(read) if read > 0) {}
}

fn run(commands: Receiver<Command>, wake: OwnedFd, finished: Sender<Done>, ping: Ping) {
    let now = Instant::now();
    let mut live = Transfers {
        readers: Vec::new(),
        writers: Vec::new(),
        generation: 0,
        sent: 0,
        deadline: now,
        idle_check: now,
    };
    loop {
        live.wait(&wake);
        clear(&wake);
        loop {
            match commands.try_recv() {
                Ok(command) => live.apply(command),
                Err(TryRecvError::Empty) => break,
                // The compositor dropped its handle. Closing the wakeup pipe is also what
                // ends the `poll` above, so this is reached promptly at shutdown.
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // Both directions every wake: a stalled paste must not stop the capture pass from
        // running, nor the other way round.
        live.pull(&finished, &ping);
        live.push();
    }
}
