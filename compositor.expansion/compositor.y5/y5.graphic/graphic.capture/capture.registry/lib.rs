//! # y5_compositor_capture_registry
//!
//! Mechanical capture engine for compositor-side screen capture into
//! dmabuf-backed textures. Knows nothing about Bevy, Wayland scenes, or
//! consumer-specific concerns.
//!
//! ## Two object types
//!
//! - [`CaptureHandle`] — a reference to a *continuous* capture managed by
//!   the registry. The registry blits into its dmabuf every tick. Cloneable
//!   (multiple consumers share the live feed). Auto-deduplicated by
//!   `(source, size)`.
//!
//! - [`SnapshotHandle`] — a *frozen* dmabuf owned independently of the
//!   registry. Returned by [`CaptureHandle::take`] or [`CaptureHandle::snapshot`].
//!   Cloneable (multiple consumers share the same frozen image). Not
//!   managed by the registry — drops when the last clone is released.
//!
//! ## The take optimization
//!
//! If a `CaptureHandle` is the sole reference to its underlying entry,
//! `take` transfers the dmabuf zero-copy into a `SnapshotHandle`. The
//! registry stops blitting (the entry is gone). This is the common
//! lock-screen case.
//!
//! If the handle is shared with other clones, `take` allocates a fresh
//! dmabuf, blits the current contents into it, and returns that. Other
//! clones keep their live stream.

#[macro_use]
extern crate compositor_model_debug_instance_record;

mod entry;
mod error;
mod handle;
pub mod registry;
mod snapshot;
mod source;

pub use entry::EntryId;
pub use error::CaptureError;
pub use handle::CaptureHandle;
pub use registry::CaptureRegistry;
pub use snapshot::SnapshotHandle;
pub use source::{CaptureSource, OutputId};
