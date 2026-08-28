//! `LaunchWorker`: the handle held in the executor for off-thread launches.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use smithay::reexports::calloop::channel::Sender as CalloopSender;

use compositor_introspection_execution_launch_execute::execute::execute;
use compositor_introspection_execution_launch_reap::reap::reap_zombies;
use compositor_introspection_execution_launch_types::types::{LaunchOutcome, LaunchRequest};

/// How hard a reap tries for the spawn interlock. A spawn holds it for a fork
/// plus an exec handshake, so losing it means another thread is mid-spawn.
const REAP_ATTEMPTS: usize = 8;
const REAP_BACKOFF: Duration = Duration::from_millis(2);

/// What the worker thread does. Reaping rides the same queue BECAUSE the two
/// must not overlap: one thread doing both serialises them by construction, and
/// keeps the calloop thread out of the interlock — it posts and returns.
enum Job {
    Launch(LaunchRequest),
    Reap,
}

/// Submit launches off the calloop thread. Cloneable; the worker thread lives
/// for the process lifetime. `scope` (whether to adopt into a systemd scope) is
/// fixed at spawn — it depends only on systemd availability, not per-launch.
#[derive(Clone)]
pub struct LaunchWorker {
    tx: mpsc::Sender<Job>,
}

impl LaunchWorker {
    /// Spawn the worker thread. `outcomes` is the calloop side; the caller must
    /// insert its receiver as a loop source and dispatch each outcome.
    pub fn spawn(outcomes: CalloopSender<LaunchOutcome>, scope: bool) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("y5-launch".into())
            .spawn(move || run(rx, outcomes, scope))
            .unwrap_or_else(|e| abort!("spawn launch worker: {e:?}"));
        Self { tx }
    }

    /// Queue a launch. A gone worker is REPORTED, not swallowed: this thread is
    /// the queue's only consumer, so its absence means nothing will ever spawn
    /// again — which looks exactly like an app declining to open a window.
    pub fn submit(&self, req: LaunchRequest) {
        let program = req.argv.first().cloned();
        if self.tx.send(Job::Launch(req)).is_err() {
            error!("launch worker gone; launch dropped: {program:?}");
        }
    }

    /// Queue a reap (the SIGCHLD source's whole job). Silent on a gone worker:
    /// `submit` already reports that, and one report is enough.
    pub fn reap(&self) {
        let _ = self.tx.send(Job::Reap);
    }
}

fn run(rx: mpsc::Receiver<Job>, outcomes: CalloopSender<LaunchOutcome>, scope: bool) {
    while let Ok(job) = rx.recv() {
        let req = match job {
            // Retry briefly while another thread's spawn holds the interlock.
            // Sleeping here is free: this thread's only deadline is the next
            // launch, and one queued behind this is one nobody has asked for yet.
            Job::Reap => {
                for attempt in 0..REAP_ATTEMPTS {
                    if reap_zombies().is_some() {
                        break;
                    }
                    if attempt + 1 < REAP_ATTEMPTS {
                        thread::sleep(REAP_BACKOFF);
                    }
                }
                continue;
            }
            Job::Launch(req) => req,
        };
        // One launch may not take the queue down with it. The interlock closes the
        // spawn/reap race that used to panic this thread — silently disabling every
        // launch for the rest of the session; this keeps ANY future panic in the
        // spawn path costing one launch rather than all of them.
        let outcome = match catch_unwind(AssertUnwindSafe(|| execute(&req, scope))) {
            Ok(outcome) => outcome,
            Err(_) => {
                error!("launch panicked: {:?}", req.argv.first());
                let result = Err(String::from("launch panicked"));
                LaunchOutcome { correlation: req.correlation, token: req.token.clone(), pid: None, result }
            }
        };
        if outcomes.send(outcome).is_err() {
            break; // calloop receiver dropped → the loop is shutting down.
        }
    }
}
