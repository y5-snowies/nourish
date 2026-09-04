//! `LaunchWorker`: the handle held in the executor for off-thread launches.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread;

use smithay::reexports::calloop::channel::Sender as CalloopSender;

use compositor_introspection_execution_launch_execute::execute::execute;
use compositor_introspection_execution_launch_types::types::{LaunchOutcome, LaunchRequest};

/// What the worker thread does — one thing, now that reaping is the loop's job.
///
/// It used to carry a `Reap` variant too, because reaping meant `waitpid(-1)` and that
/// had to be serialised against every spawn in the process. A child is now named by its
/// own descriptor, so there is nothing to serialise and nothing to post here.
enum Job {
    Launch(LaunchRequest),
}

/// Submit launches off the calloop thread. Cloneable; the worker thread lives
/// for the process lifetime. `scope` (whether to adopt into a systemd scope) is
/// fixed at spawn — it depends only on systemd availability, not per-launch.
#[derive(Clone)]
pub struct LaunchWorker {
    tx: mpsc::Sender<Job>,
}

impl LaunchWorker {
    /// Spawn the worker thread. `outcomes` is the calloop side; the caller must insert
    /// its receiver as a loop source and dispatch each outcome. Exit descriptors do not
    /// travel this way: `spawn_detached` hands each one to the sink the loader installed
    /// (`child.pidfd`), so nothing about reaping is threaded through here.
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

}

fn run(rx: mpsc::Receiver<Job>, outcomes: CalloopSender<LaunchOutcome>, scope: bool) {
    while let Ok(job) = rx.recv() {
        let req = match job {
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
