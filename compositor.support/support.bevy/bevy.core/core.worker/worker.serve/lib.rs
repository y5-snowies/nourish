//! The worker thread: owns every `App`, every ring, and the wgpu work.
//!
//! Nothing here runs on the compositor thread, so a heavy bevy frame can no
//! longer delay input. Jobs arrive on a channel; finished buffers leave through
//! the `Board`. The loop BLOCKS between compositor frames rather than spinning —
//! `Job::Tick` is the only thing that advances a scene, and it is coalesced, so a
//! backlog of ticks queued while the GPU was busy still costs exactly one frame.

pub mod serve;
pub use serve::*;
