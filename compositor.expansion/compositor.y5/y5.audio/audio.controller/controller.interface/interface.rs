//! Call-site-agnostic control of the **default audio sink** (output) via the
//! PulseAudio client API. On Fedora / most modern distros this is served
//! transparently by PipeWire's `pipewire-pulse`, so the same code drives both.
//!
//! Capabilities:
//!   * On-demand polling          -> [`AudioController::refresh`] / [`AudioController::state`]
//!   * Change subscriptions        -> [`AudioController::watch`] hands each call site
//!                                   its own [`AudioWatch`] that is *pinged* on every
//!                                   change; the owner re-polls and unsubscribes
//!                                   implicitly when the watch is dropped.
//!   * Actions                    -> set / adjust volume, set / toggle mute
//!   * Background watcher          -> started by [`AudioController::new`]; on every
//!                                   audio-topology change (including edits made
//!                                   outside this process) it pings all [`AudioWatch`]ers.
//!
//! The PulseAudio threaded mainloop *is* the off-thread component, and it lives
//! entirely inside this module — the caller never spawns a thread. The watcher
//! pings from that mainloop thread via a non-blocking, coalescing capacity-1
//! channel, so it never stalls waiting on a receiver; the subscriber list is
//! behind a `Mutex`, and each owner re-reads `state()` on its own thread.
//!
//! This type is intentionally not `Send`/`Sync`: create it and call its methods
//! from one thread (your compositor's main/event-loop thread). PulseAudio does
//! its own work on its internal mainloop thread; the shared snapshot it updates
//! is behind an `Arc<Mutex<_>>`, so reading `state()` is always cheap and safe.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::introspect::Introspector;
use pulse::context::subscribe::{Facility, InterestMaskSet};
use pulse::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use pulse::mainloop::threaded::Mainloop;
use pulse::operation::{Operation, State as OpState};
use pulse::volume::{ChannelVolumes, Volume};

/// A snapshot of the default sink. `volume` is a fraction where `1.0` == 100%
/// (`PA_VOLUME_NORM`). It can exceed `1.0` if you allow boost via
/// [`AudioController::set_max_volume`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioState {
    pub sink_name: Option<String>,
    pub description: Option<String>,
    pub volume: f64,
    pub muted: bool,
    /// All output sinks (for the settings Audio tab); the default has `is_default`.
    pub sinks: Vec<SinkInfo>,
}

/// One output sink for the Audio tab list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SinkInfo {
    pub name: String,
    pub description: String,
    pub volume: f64,
    pub muted: bool,
    pub is_default: bool,
}

#[derive(Debug)]
pub enum AudioError {
    /// Could not allocate the PulseAudio mainloop.
    Mainloop,
    /// Could not allocate the PulseAudio context.
    Context,
    /// `pa_context_connect` returned an error.
    Connect(pulse::error::PAErr),
    /// The context reached the Failed/Terminated state during connect.
    ConnectFailed,
    /// No default sink is currently available (e.g. boot-time race).
    NoDefaultSink,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Mainloop => write!(f, "failed to create PulseAudio mainloop"),
            AudioError::Context => write!(f, "failed to create PulseAudio context"),
            AudioError::Connect(e) => write!(f, "failed to connect to PulseAudio: {e}"),
            AudioError::ConnectFailed => write!(f, "PulseAudio context failed to become ready"),
            AudioError::NoDefaultSink => write!(f, "no default sink available"),
        }
    }
}
impl std::error::Error for AudioError {}

pub struct AudioController {
    mainloop: Rc<RefCell<Mainloop>>,
    context: Rc<RefCell<Context>>,
    introspect: Introspector,
    state: Arc<Mutex<AudioState>>,
    /// Live subscribers. Shared with the PulseAudio mainloop-thread subscribe
    /// callback, which pings every sender on a change — so this is touched from
    /// two threads and MUST be a `Mutex`, not a `RefCell`. Each entry is keyed by
    /// an id so its [`AudioWatch`] can remove exactly itself on drop.
    watchers: Arc<Mutex<Watchers>>,
    /// Upper bound for set/adjust, as a fraction of NORMAL. `1.0` caps at 100%
    /// (safer UX); set to e.g. `1.5` to allow boost like most desktops.
    max_volume: f64,
}

/// Subscriber registry behind [`AudioController::watchers`]. Each sender is a
/// capacity-1 [`SyncSender`] used as a coalescing ping: the callback `try_send`s
/// a `()` and drops it on `Full` (a ping is already pending), so notifying is
/// non-blocking and never rendezvous — the audio thread is never stalled.
#[derive(Default)]
struct Watchers {
    next_id: u64,
    subs: Vec<(u64, SyncSender<()>)>,
}

/// A live subscription to [`AudioController`], handed out by
/// [`AudioController::watch`]. It carries *pings*, not state: when the audio
/// topology changes the controller wakes every watch, and the owner re-reads
/// [`AudioController::state`] / [`AudioController::refresh`] itself. Each call
/// site owns one independently; dropping it deregisters — no manual unsubscribe.
pub struct AudioWatch {
    id: u64,
    rx: Receiver<()>,
    watchers: Arc<Mutex<Watchers>>,
}

impl AudioWatch {
    /// Flush the ping channel and report whether we were pinged since the last
    /// call. Coalescing: any number of pings collapse to a single `true` (you
    /// re-poll once regardless). Cheap and non-blocking; returns `false` on quiet
    /// ticks. A fresh subscription is pre-pinged, so the first call returns `true`
    /// to prompt an initial poll.
    pub fn pinged(&self) -> bool {
        let mut pinged = false;
        while self.rx.try_recv().is_ok() {
            pinged = true;
        }
        pinged
    }
}

impl Drop for AudioWatch {
    fn drop(&mut self) {
        // Explicit deregistration: remove exactly our sender. Poisoned lock → the
        // controller is tearing down anyway, nothing to clean up.
        if let Ok(mut w) = self.watchers.lock() {
            w.subs.retain(|(id, _)| *id != self.id);
        }
    }
}

impl AudioController {
    /// Connect to the default PulseAudio/PipeWire server and start the
    /// background mainloop. Blocks until the connection is ready (or fails).
    pub fn new(app_name: &str) -> Result<Self, AudioError> {
        let mainloop = Rc::new(RefCell::new(Mainloop::new().ok_or(AudioError::Mainloop)?));
        let context = Rc::new(RefCell::new(
            Context::new(&*mainloop.borrow(), app_name).ok_or(AudioError::Context)?,
        ));

        // The context state callback signals the mainloop so our `wait()` loop
        // below wakes up on every state transition. We use raw pointers inside
        // the callback to side-step RefCell's borrow flag (the callback runs on
        // the mainloop thread; PulseAudio serialises access internally).
        {
            let ml = Rc::clone(&mainloop);
            let ctx = Rc::clone(&context);
            context
                .borrow_mut()
                .set_state_callback(Some(Box::new(move || {
                    let st = unsafe { (*ctx.as_ptr()).get_state() };
                    if matches!(
                        st,
                        ContextState::Ready | ContextState::Failed | ContextState::Terminated
                    ) {
                        unsafe { (*ml.as_ptr()).signal(false) };
                    }
                })));
        }

        context
            .borrow_mut()
            .connect(None, ContextFlagSet::NOFLAGS, None)
            .map_err(AudioError::Connect)?;

        mainloop.borrow_mut().lock();
        mainloop
            .borrow_mut()
            .start()
            .map_err(|_| AudioError::ConnectFailed)?;

        // Wait until ready.
        loop {
            let st = context.borrow().get_state();
            match st {
                ContextState::Ready => break,
                ContextState::Failed | ContextState::Terminated => {
                    mainloop.borrow_mut().unlock();
                    mainloop.borrow_mut().stop();
                    return Err(AudioError::ConnectFailed);
                }
                _ => mainloop.borrow_mut().wait(),
            }
        }
        // Drop the connect-time callback; the watcher (if started) installs its own.
        context.borrow_mut().set_state_callback(None);

        let introspect = context.borrow().introspect();
        mainloop.borrow_mut().unlock();

        let me = AudioController {
            mainloop,
            context,
            introspect,
            state: Arc::new(Mutex::new(AudioState::default())),
            watchers: Arc::new(Mutex::new(Watchers::default())),
            max_volume: 1.0,
        };

        // Seed the cache so callers have a value immediately. A missing sink at
        // construction time is not fatal — it may appear later.
        let _ = me.refresh();
        // Subscribe to sink/server changes: the callback re-queries and pushes the
        // new snapshot to every watcher, all on the PulseAudio mainloop thread.
        me.install_watcher();
        Ok(me)
    }

    /// Cap for set/adjust as a fraction of NORMAL (1.0 = 100%). Default `1.0`.
    pub fn set_max_volume(&mut self, max: f64) {
        self.max_volume = max.max(0.0);
    }

    /// The last known snapshot. Cheap; never touches PulseAudio.
    pub fn state(&self) -> AudioState {
        self.state.lock().unwrap().clone()
    }

    /// Subscribe to audio-topology changes. Every call site gets its own
    /// independent [`AudioWatch`] that is *pinged* — not fed state — whenever a
    /// sink/default-sink changes, including edits made outside this process. On a
    /// ping the owner re-reads [`state`] / [`refresh`] itself. The subscription is
    /// pre-pinged so the owner polls once on start. Drop the `AudioWatch` to
    /// unsubscribe.
    ///
    /// Pings are driven only by the live sink controller, never by the action
    /// methods ([`set_sink_mute`] &c.) — a change reflects when the controller
    /// reports it live, not optimistically because the caller "knows" it.
    ///
    /// [`state`]: AudioController::state
    /// [`refresh`]: AudioController::refresh
    /// [`set_sink_mute`]: AudioController::set_sink_mute
    pub fn watch(&self) -> AudioWatch {
        // Capacity 1: holds at most one pending ping (extras coalesce), and
        // `try_send` never blocks — so the PulseAudio thread is never stalled.
        let (tx, rx) = sync_channel(1);
        let _ = tx.try_send(()); // pre-ping: poll once on subscribe
        let mut w = self.watchers.lock().unwrap();
        let id = w.next_id;
        w.next_id += 1;
        w.subs.push((id, tx));
        AudioWatch { id, rx, watchers: Arc::clone(&self.watchers) }
    }

    /// Synchronously re-query the default sink (+ the full sink list) and update
    /// the cache. Does *not* broadcast — the live watcher is the only publisher —
    /// so this is for on-demand reads / seeding, not for notifying subscribers.
    pub fn refresh(&self) -> Result<AudioState, AudioError> {
        let mut snap = self.query_default_sink()?;
        let default = snap.sink_name.clone().unwrap_or_default();
        snap.sinks = self.query_sinks(&default);
        *self.state.lock().unwrap() = snap.clone();
        Ok(snap)
    }

    /// Make `name` the default output sink.
    pub fn set_default_sink(&self, name: &str) -> Result<AudioState, AudioError> {
        self.mainloop.borrow_mut().lock();
        let ml = Rc::clone(&self.mainloop);
        let op = self.context.borrow_mut().set_default_sink(name, move |_ok| {
            unsafe { (*ml.as_ptr()).signal(false) };
        });
        while op.get_state() == OpState::Running {
            self.mainloop.borrow_mut().wait();
        }
        self.mainloop.borrow_mut().unlock();
        self.refresh()
    }

    /// Set a specific sink's volume (fraction of NORMAL), preserving balance.
    pub fn set_sink_volume(&self, name: &str, fraction: f64) -> Result<AudioState, AudioError> {
        let target = fraction.clamp(0.0, self.max_volume);
        let mut cv = self.query_sink_volumes(name)?;
        let target_vol = Volume((target * Volume::NORMAL.0 as f64).round() as u32);
        if cv.scale(target_vol).is_none() {
            cv.set(cv.len().max(1), target_vol);
        }
        let name = name.to_string();
        self.run(|introspect, done| introspect.set_sink_volume_by_name(&name, &cv, Some(done)))?;
        self.refresh()
    }

    /// Set a specific sink's mute state.
    pub fn set_sink_mute(&self, name: &str, muted: bool) -> Result<AudioState, AudioError> {
        let name = name.to_string();
        self.run(|introspect, done| introspect.set_sink_mute_by_name(&name, muted, Some(done)))?;
        self.refresh()
    }

    /// Per-channel volumes of a named sink (synchronous).
    fn query_sink_volumes(&self, name: &str) -> Result<ChannelVolumes, AudioError> {
        let out: Rc<RefCell<Option<ChannelVolumes>>> = Rc::new(RefCell::new(None));
        let name = name.to_string();
        {
            let out = Rc::clone(&out);
            let ml = Rc::clone(&self.mainloop);
            self.run_op(move |introspect| {
                introspect.get_sink_info_by_name(&name, move |res| match res {
                    ListResult::Item(info) => *out.borrow_mut() = Some(info.volume),
                    _ => unsafe { (*ml.as_ptr()).signal(false) },
                })
            });
        }
        let v = out.borrow_mut().take();
        v.ok_or(AudioError::NoDefaultSink)
    }

    /// All sinks (synchronous), marking the one equal to `default_name`.
    fn query_sinks(&self, default_name: &str) -> Vec<SinkInfo> {
        let out: Rc<RefCell<Vec<SinkInfo>>> = Rc::new(RefCell::new(Vec::new()));
        let dn = default_name.to_string();
        {
            let out = Rc::clone(&out);
            let ml = Rc::clone(&self.mainloop);
            self.run_op(move |introspect| {
                introspect.get_sink_info_list(move |res| match res {
                    ListResult::Item(info) => {
                        let name = info.name.as_ref().map(|c| c.to_string()).unwrap_or_default();
                        out.borrow_mut().push(SinkInfo {
                            is_default: name == dn,
                            name,
                            description: info.description.as_ref().map(|c| c.to_string()).unwrap_or_default(),
                            volume: info.volume.max().0 as f64 / Volume::NORMAL.0 as f64,
                            muted: info.mute,
                        });
                    }
                    _ => unsafe { (*ml.as_ptr()).signal(false) },
                })
            });
        }
        let v = out.borrow().clone();
        v
    }

    /// Set absolute volume as a fraction of NORMAL (`0.0..=max_volume`). Balance
    /// between channels is preserved by scaling the existing per-channel volumes.
    pub fn set_volume(&self, fraction: f64) -> Result<AudioState, AudioError> {
        let target = fraction.clamp(0.0, self.max_volume);
        let (name, mut cv, _mute) = self.query_raw_default_sink()?;
        let target_vol = Volume((target * Volume::NORMAL.0 as f64).round() as u32);
        // scale() sets the loudest channel to `target_vol` and scales the rest
        // proportionally, preserving L/R balance. Falls back to a flat set.
        if cv.scale(target_vol).is_none() {
            cv.set(cv.len().max(1), target_vol);
        }
        self.run(|introspect, done| {
            introspect.set_sink_volume_by_name(&name, &cv, Some(done))
        })?;
        self.refresh()
    }

    /// Relative volume change, e.g. `+0.05` for +5%, `-0.05` for -5%.
    pub fn adjust_volume(&self, delta: f64) -> Result<AudioState, AudioError> {
        let cur = self.refresh()?.volume;
        self.set_volume(cur + delta)
    }

    pub fn set_muted(&self, muted: bool) -> Result<AudioState, AudioError> {
        let (name, _cv, _m) = self.query_raw_default_sink()?;
        self.run(|introspect, done| {
            introspect.set_sink_mute_by_name(&name, muted, Some(done))
        })?;
        self.refresh()
    }

    pub fn toggle_mute(&self) -> Result<AudioState, AudioError> {
        let cur = self.refresh()?.muted;
        self.set_muted(!cur)
    }

    /// Subscribe to sink/server changes and, on each one, *ping* every [`watch`]er
    /// so the owner re-polls. Runs on the PulseAudio mainloop thread; it does no
    /// querying and never blocks — each ping is a coalescing `try_send` that is
    /// dropped if one is already pending. Started once by [`new`].
    ///
    /// [`watch`]: AudioController::watch
    /// [`new`]: AudioController::new
    fn install_watcher(&self) {
        let watchers = Arc::clone(&self.watchers);

        self.mainloop.borrow_mut().lock();
        self.context
            .borrow_mut()
            .set_subscribe_callback(Some(Box::new(move |facility, _op, _idx| {
                // Any output sink change, or a default-sink (server) change.
                if !matches!(facility, Some(Facility::Sink) | Some(Facility::Server)) {
                    return;
                }
                if let Ok(w) = watchers.lock() {
                    for (_, tx) in &w.subs {
                        // Non-blocking, coalescing: `Full` just means a ping is
                        // already queued; `Disconnected` means a dropped watch not
                        // yet reaped — both are fine to ignore.
                        let _ = tx.try_send(());
                    }
                }
            })));
        self.context
            .borrow_mut()
            .subscribe(InterestMaskSet::SINK | InterestMaskSet::SERVER, |_success| {});
        self.mainloop.borrow_mut().unlock();
    }

    // ----- internal synchronous helpers -----

    /// Resolve the default sink name + current per-channel volumes + mute,
    /// synchronously. Returns raw `ChannelVolumes` so callers can preserve balance.
    fn query_raw_default_sink(&self) -> Result<(String, ChannelVolumes, bool), AudioError> {
        let name = self.query_default_sink_name()?;
        let out: Rc<RefCell<Option<(ChannelVolumes, bool)>>> = Rc::new(RefCell::new(None));
        {
            let out = Rc::clone(&out);
            let ml = Rc::clone(&self.mainloop);
            self.run_op(|introspect| {
                let out = Rc::clone(&out);
                let ml = Rc::clone(&ml);
                introspect.get_sink_info_by_name(&name, move |res| {
                    if let ListResult::Item(info) = res {
                        *out.borrow_mut() = Some((info.volume, info.mute));
                    }
                    if !matches!(res, ListResult::Item(_)) {
                        unsafe { (*ml.as_ptr()).signal(false) };
                    }
                })
            });
        }
        let (cv, mute) = out.borrow_mut().take().ok_or(AudioError::NoDefaultSink)?;
        Ok((name, cv, mute))
    }

    fn query_default_sink(&self) -> Result<AudioState, AudioError> {
        let name = self.query_default_sink_name()?;
        let out: Rc<RefCell<Option<AudioState>>> = Rc::new(RefCell::new(None));
        {
            let out = Rc::clone(&out);
            let ml = Rc::clone(&self.mainloop);
            self.run_op(|introspect| {
                let out = Rc::clone(&out);
                let ml = Rc::clone(&ml);
                introspect.get_sink_info_by_name(&name, move |res| {
                    match res {
                        ListResult::Item(info) => *out.borrow_mut() = Some(sink_info_to_state(info)),
                        _ => unsafe { (*ml.as_ptr()).signal(false) },
                    }
                })
            });
        }
        let result = out.borrow_mut().take().ok_or(AudioError::NoDefaultSink);
        result
    }

    fn query_default_sink_name(&self) -> Result<String, AudioError> {
        let out: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let out = Rc::clone(&out);
            let ml = Rc::clone(&self.mainloop);
            self.run_op(|introspect| {
                let out = Rc::clone(&out);
                let ml = Rc::clone(&ml);
                introspect.get_server_info(move |info| {
                    *out.borrow_mut() = info.default_sink_name.as_ref().map(|c| c.to_string());
                    unsafe { (*ml.as_ptr()).signal(false) };
                })
            });
        }
        let result = out.borrow_mut().take().ok_or(AudioError::NoDefaultSink);
        result
    }

    /// Run an introspect operation to completion under the mainloop lock,
    /// driving `wait()` until the operation leaves the Running state.
    fn run_op<T: ?Sized, B>(&self, build: B)
    where
        B: FnOnce(&Introspector) -> Operation<T>,
    {
        self.mainloop.borrow_mut().lock();
        let op = build(&self.introspect);
        while op.get_state() == OpState::Running {
            self.mainloop.borrow_mut().wait();
        }
        self.mainloop.borrow_mut().unlock();
    }

    /// Run a "set" style operation (callback is `FnMut(bool)` reporting success)
    /// to completion, signalling the mainloop from the success callback.
    fn run<B>(&self, build: B) -> Result<(), AudioError>
    where
        B: FnOnce(&mut Introspector, Box<dyn FnMut(bool) + 'static>) -> Operation<dyn FnMut(bool)>,
    {
        self.mainloop.borrow_mut().lock();
        let mut introspect = self.context.borrow().introspect();
        let ml = Rc::clone(&self.mainloop);
        let done: Box<dyn FnMut(bool) + 'static> = Box::new(move |_success| {
            unsafe { (*ml.as_ptr()).signal(false) };
        });
        let op = build(&mut introspect, done);
        while op.get_state() == OpState::Running {
            self.mainloop.borrow_mut().wait();
        }
        self.mainloop.borrow_mut().unlock();
        Ok(())
    }
}

impl Drop for AudioController {
    fn drop(&mut self) {
        self.mainloop.borrow_mut().lock();
        self.context.borrow_mut().set_subscribe_callback(None);
        self.context.borrow_mut().disconnect();
        self.mainloop.borrow_mut().unlock();
        self.mainloop.borrow_mut().stop();
    }
}

fn sink_info_to_state(info: &pulse::context::introspect::SinkInfo) -> AudioState {
    AudioState {
        sink_name: info.name.as_ref().map(|c| c.to_string()),
        description: info.description.as_ref().map(|c| c.to_string()),
        volume: info.volume.max().0 as f64 / Volume::NORMAL.0 as f64,
        muted: info.mute,
        sinks: Vec::new(),
    }
}

