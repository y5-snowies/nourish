use std::collections::VecDeque;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::channel::Sender as CalloopSender;
use compositor_introspection_extraction_window_base::{extract_hints, refresh_meta_from_pid, HandlerRegistry};
use compositor_introspection_inference_hint_base::ApplicationData;
use compositor_introspection_sampler_window_batch::batch::{flush, SampleBatch, SampleResult};
use compositor_introspection_sampler_window_schedule::schedule::{
    apply_registration, tick_interval, Entry, Registration, BATCH_CAP, QUICK_DEBOUNCE, SLOW_DEBOUNCE,
};

pub fn run(
    rx: mpsc::Receiver<Registration>,
    registry: Arc<HandlerRegistry>,
    results_tx: CalloopSender<SampleBatch>,
) {
    let mut queue: VecDeque<Entry> = VecDeque::new();
    let mut buffer: Vec<SampleResult> = Vec::new();
    let mut flush_deadline: Option<Instant> = None;
    let mut last_flush_time = Instant::now() - SLOW_DEBOUNCE - Duration::from_secs(1);
    let mut next_sample_time = Instant::now();
    // How many entries an explicit `Refresh` asked for since the last pass. It
    // widens the NEXT pass past BATCH_CAP and pulls it forward to now, so one
    // "sample every window on this world" request lands in a single batch
    // instead of trickling out over the background cadence.
    let mut forced: usize = 0;

    loop {
        while let Ok(reg) = rx.try_recv() {
            forced += apply_registration(&mut queue, reg);
        }
        if forced > 0 {
            next_sample_time = Instant::now();
        }

        if queue.is_empty() {
            if !buffer.is_empty() {
                if !flush(&mut buffer, &results_tx, Instant::now()) {
                    return;
                }
                last_flush_time = Instant::now();
                flush_deadline = None;
            }
            match rx.recv() {
                Ok(reg) => {
                    forced += apply_registration(&mut queue, reg);
                    next_sample_time = Instant::now();
                    continue;
                }
                Err(_) => return, // registration channel closed: shutdown
            }
        }

        let now = Instant::now();
        let mut next_wake = next_sample_time;
        if let Some(d) = flush_deadline {
            if d < next_wake {
                next_wake = d;
            }
        }
        match rx.recv_timeout(next_wake.saturating_duration_since(now)) {
            Ok(reg) => {
                forced += apply_registration(&mut queue, reg);
                if forced > 0 {
                    next_sample_time = Instant::now();
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();

        if now >= next_sample_time {
            let was_empty = buffer.is_empty();
            let this_pass = std::mem::take(&mut forced);
            for _ in 0..queue.len().min(BATCH_CAP.max(this_pass)) {
                let Some(entry) = queue.pop_front() else { break };
                match refresh_meta_from_pid(entry.pid, &entry.previous_meta) {
                    Some(meta) => {
                        let hints = extract_hints(&meta, &registry);
                        let data = ApplicationData::new(meta.clone(), hints);
                        queue.push_back(Entry { uuid: entry.uuid, pid: entry.pid, previous_meta: meta });
                        buffer.push(SampleResult { uuid: entry.uuid, data: Some(data) });
                    }
                    None => buffer.push(SampleResult { uuid: entry.uuid, data: None }),
                }
            }
            next_sample_time = now + tick_interval(queue.len());

            if was_empty && !buffer.is_empty() && flush_deadline.is_none() {
                let quiet = now.duration_since(last_flush_time) > SLOW_DEBOUNCE;
                flush_deadline = Some(now + if quiet { QUICK_DEBOUNCE } else { SLOW_DEBOUNCE });
            }
            // A requested pass is being waited on by a UI, so it does not get the
            // steady-state debounce: pull any pending deadline in to QUICK.
            if this_pass > 0 && !buffer.is_empty() {
                let soon = now + QUICK_DEBOUNCE;
                flush_deadline = Some(flush_deadline.map_or(soon, |d| d.min(soon)));
            }
        }

        if let Some(d) = flush_deadline {
            if now >= d {
                if !flush(&mut buffer, &results_tx, now) {
                    return;
                }
                last_flush_time = now;
                flush_deadline = None;
            }
        }
    }
}
