//! Start the bevy worker thread.
//!
//! Returns `None` when the thread cannot be started, and the caller stays on the
//! inline path — unlike the background worker, whose failure deliberately leaves
//! the background absent. The difference is what is at stake: a missing shader
//! backdrop is cosmetic, a missing lock screen or overview is not.

pub mod spawn;
pub use spawn::*;
