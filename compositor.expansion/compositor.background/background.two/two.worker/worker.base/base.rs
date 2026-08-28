//! The background render worker: a thread owning its own Vulkan device, turning
//! per-pane draw signals into finished dmabufs the compositor samples.
//!
//! Built as if it were a separate process. The compositor shares no Vulkan
//! object, no command buffer, and no mutable state with it — the channels are the
//! draw signals in ([`Signal`]), the published slots out (the pane [`Registry`])
//! and the host-side results out (`pane::Readback`). Moving this to a real
//! process later is mechanical.
//!
//! ONE worker, MANY panes. Panes cannot share a buffer: each carries its own
//! camera and physical size, so on a multi-monitor desktop a shared buffer would
//! be the wrong resolution and the wrong view for every pane but one. They do
//! share the device and pipeline cache — the shader is the same and the GPU has
//! one queue, so separating those would buy nothing.
//!
//! Pane buffers are allocated lazily on first ping and reallocated when that
//! pane's size changes, so resize needs no involvement from the caller.

use compositor_background_two_worker_key::key::PaneKey;
use compositor_background_two_worker_pane::pane::{self, Registry};
use compositor_background_two_worker_signal::signal::{DrawRequest, Signal};
use smithay::backend::allocator::dmabuf::Dmabuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

pub use compositor_background_two_worker_signal::signal::DrawRequest as Request;

pub struct Worker {
    signal: Arc<Signal>,
    registry: Registry,
    /// The worker's return path — see `pane::Readback`. A field, not a global:
    /// this `Worker` is already an `Arc` the compositor holds.
    readbacks: compositor_background_two_worker_pane::pane::Readbacks,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Start the worker. Everything Vulkan is created ON the worker thread, so no
    /// Vulkan object is ever constructed on one thread and used from another.
    ///
    /// There is no fallback: if setup fails this returns `Err` and the caller
    /// must leave the background absent. Silently dropping back to the inline
    /// shader path would hide exactly the condition this worker exists to fix.
    /// Takes no configuration: the render format is resolved on the worker
    /// thread from the physical device and the session's achieved scanout depth
    /// (`worker.format`), and every tunable is re-read from the global each pass.
    pub fn spawn(formats: compositor_kernel_graphic_format_registrar_base::registrar::Registrar) -> Result<Self, String> {
        let signal = Arc::new(Signal::new());
        let registry: Registry = Arc::new(Mutex::new(Default::default()));
        let readbacks: compositor_background_two_worker_pane::pane::Readbacks =
            Arc::new(Mutex::new(Default::default()));
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let (s, r) = (Arc::clone(&signal), Arc::clone(&registry));
        let rb = Arc::clone(&readbacks);
        let thread = std::thread::Builder::new()
            .name("y5-background".into())
            .spawn(move || compositor_background_two_worker_serve::serve::run(formats, s, r, rb, tx))
            .map_err(|e| format!("background worker thread: {e}"))?;
        rx.recv()
            .map_err(|_| "background worker died during setup".to_string())??;
        // Settings changes are the one input that is neither a ping nor an ack,
        // and a `slots` change is a reallocation — so wake on them rather than
        // letting the next backstop notice.
        let woken = Arc::clone(&signal);
        compositor_model_environment_background_base::base::set_change_waker(Arc::new(move || {
            woken.poke()
        }));
        info!("background worker: ready");
        Ok(Self { signal, registry, readbacks, thread: Some(thread) })
    }

    /// Compositor thread: publish `pane`'s latest state and mark it live. This is
    /// NOT a render request — the worker runs at its own rate and only needs the
    /// ping to know the pane still exists, and to be woken if it had parked.
    pub fn ping(&self, pane: &PaneKey, req: DrawRequest) {
        self.signal.ping(pane, req);
    }

    /// Compositor thread: what this pane's last render computed for us.
    pub fn readback(
        &self,
        pane: &PaneKey,
    ) -> Option<compositor_background_two_worker_pane::pane::Readback> {
        self.readbacks.lock().ok()?.get(pane).cloned()
    }

    /// Compositor thread: this pane's newest fully-rendered buffer plus the
    /// generation that produced it, or `None` before its first frame completes.
    /// The generation is the compositor's damage key — while it is unchanged the
    /// background is byte-identical and needs no re-compositing.
    pub fn latest(&self, pane: &PaneKey) -> Option<(Dmabuf, usize)> {
        let out = pane::latest(&self.registry, pane);
        // Taking a frame IS the acknowledgement the worker's `drained()` gate is
        // waiting on. Telling it here turns a polled wait into an event; a no-op
        // whenever nobody is parked on it.
        if out.is_some() {
            self.signal.acknowledged();
        }
        out
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.signal.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
