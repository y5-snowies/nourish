//! Per-pipe redraw scheduling: what each output has been asked to render, what
//! it has rendered, and whether a flip of its own is in flight.
//!
//! A redraw REQUEST is global — every monitor renders the same world, so a commit
//! or an input event makes every pipe stale at once. Everything after that is per
//! pipe: a pipe renders when it lags the request epoch, is skipped while its flip
//! is in flight (its own vblank re-renders it), and the loop is woken only when
//! some pipe is idle and stale. There is no latch: a ping is a stale signal by
//! nature ("a request happened since the last drain"), so the executor decides
//! from the epoch alone whether a wake has work behind it. The single-`bool`
//! latch and in-flight gate this replaces were written for one output and could
//! not say WHICH pipe a wake was for.
//!
//! Incremental by design: nothing is registered. A pipe exists from the first
//! time the kernel reports on it, an unknown pipe reads as stale and idle, and
//! `clear` is safe at any point — the price is at most one redundant render per
//! pipe. Keyed by the output key the kernel already uses (`tearing.liveness`
//! keys by it too); this layer knows nothing about CRTCs.

use smithay::reexports::calloop::ping::Ping;

struct Pipe { key: String, rendered: u64, in_flight: bool }

pub struct Schedule { epoch: u64, pipes: Vec<Pipe>, ping: Option<Ping> }

impl Schedule {
    /// Epoch 1 with every pipe first seen at 0: the first render is always due.
    pub fn new() -> Self { Self { epoch: 1, pipes: Vec::new(), ping: None } }
    pub fn set_ping(&mut self, ping: Ping) { self.ping = Some(ping); }
    pub fn epoch(&self) -> u64 { self.epoch }

    /// Forget a pruned pipe — optional hygiene, a stale entry never blocks anything.
    pub fn remove(&mut self, key: &str) { self.pipes.retain(|p| p.key != key); }
    /// Forget everything: every pipe is unknown again, i.e. stale and idle.
    pub fn clear(&mut self) { self.pipes.clear(); }

    /// A redraw request: every pipe is stale. Wakes the loop iff a pipe can act on
    /// it now; the rest are in flight and their own vblank renders them. (No pipes
    /// known yet — boot — wakes too: the first render is what creates them.)
    pub fn request(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.pipes.is_empty() || self.pipes.iter().any(|p| !p.in_flight) {
            self.wake();
        }
    }
    /// A request without a wake: for callers that run the executor themselves,
    /// and for continuation sources called from INSIDE a render (the parallax),
    /// whose pipe's vblank finds the epoch.
    pub fn request_silent(&mut self) { self.epoch = self.epoch.wrapping_add(1); }
    /// A request that wakes unconditionally — rescues and re-arms, where an idle
    /// cycle must restart whatever the bookkeeping says.
    pub fn force(&mut self) { self.request_silent(); self.wake(); }
    fn wake(&self) { if let Some(p) = &self.ping { p.ping(); } }

    /// Is there a pipe a wake can render right now — idle and behind the epoch?
    pub fn pending(&self) -> bool {
        self.pipes.iter().any(|p| !p.in_flight && p.rendered != self.epoch)
    }
    /// Unknown pipes need rendering: a first frame is never gated on bookkeeping.
    pub fn needs(&self, key: &str) -> bool {
        self.get(key).is_none_or(|p| p.rendered != self.epoch)
    }
    pub fn in_flight(&self, key: &str) -> bool { self.get(key).is_some_and(|p| p.in_flight) }

    /// A render begins: the pipe is current as of NOW. A request that arrives
    /// while it renders moves the epoch past this stamp and is serviced next time.
    pub fn rendering(&mut self, key: &str) { let e = self.epoch; self.entry(key).rendered = e; }
    /// The frame was queued; only this pipe's own vblank ends the flight.
    pub fn queued(&mut self, key: &str) { self.entry(key).in_flight = true; }
    /// The flip completed — or never can (rebuilt pipe, session resume).
    pub fn completed(&mut self, key: &str) { self.entry(key).in_flight = false; }

    fn get(&self, key: &str) -> Option<&Pipe> { self.pipes.iter().find(|p| p.key == key) }
    /// Upsert: the first report on a pipe is what creates it.
    fn entry(&mut self, key: &str) -> &mut Pipe {
        let i = match self.pipes.iter().position(|p| p.key == key) {
            Some(i) => i,
            None => {
                self.pipes.push(Pipe { key: key.to_string(), rendered: 0, in_flight: false });
                self.pipes.len() - 1
            }
        };
        &mut self.pipes[i]
    }
}
