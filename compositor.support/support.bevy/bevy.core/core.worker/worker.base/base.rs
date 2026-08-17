use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_publish_base::Board;
use compositor_support_bevy_core_shared_base::SharedContext;
use smithay::utils::{Physical, Size};
use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::Sender;

/// One instance's bevy runtime with its scene type erased, so the worker can hold
/// a heterogeneous set without itself being generic over `S: BevyScene`.
pub trait AnyRuntime: 'static {
    fn update(&mut self);
    /// Re-point the render graph at another ring slot. Called before `update`.
    fn set_output_texture(&mut self, texture: Arc<wgpu::Texture>);
    fn resize(&mut self, size: (u32, u32), scale: f32);
    /// Apply a typed scene command. The payload is `S::Command`, boxed by the
    /// registry, and the impl downcasts it back. A mismatch is a registry bug:
    /// it is logged and dropped rather than panicking the worker thread.
    fn apply(&mut self, command: Box<dyn Any + Send>);
}

/// Builds one runtime ON the worker thread, given the shared wgpu context and the
/// ring's first slot as the initial render target.
pub type Factory = Box<
    dyn FnOnce(&SharedContext, Arc<wgpu::Texture>, (u32, u32), f32) -> Box<dyn AnyRuntime> + Send,
>;

pub enum Job {
    Create { id: HandleId, size: Size<i32, Physical>, scale: f32, factory: Factory },
    Destroy(HandleId),
    Resize { id: HandleId, size: Size<i32, Physical>, scale: f32 },
    Command { id: HandleId, payload: Box<dyn Any + Send> },
    /// One compositor frame happened. Coalesced by the worker: many ticks queued
    /// while it was busy still advance the scene exactly once, which keeps bevy on
    /// the compositor's cadence without inventing a rate for it. This mirrors
    /// today's behaviour, where an instance ticks once per `render_all`.
    Tick,
}

/// The registry's end of the worker.
#[derive(Clone)]
pub struct Worker {
    tx: Sender<Job>,
    board: Board,
}

impl Worker {
    pub fn new(tx: Sender<Job>, board: Board) -> Self {
        Self { tx, board }
    }

    /// The published-frame board, read once per instance per frame when building
    /// render elements.
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Queue a job. `false` once the worker thread is gone — the caller reports
    /// that once, not per job.
    pub fn send(&self, job: Job) -> bool {
        self.tx.send(job).is_ok()
    }
}
