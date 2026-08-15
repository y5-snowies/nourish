//! A worker-owned ring slot: a dmabuf and its wgpu import, and nothing else.
//!
//! Deliberately NOT the compositor-side `Slot`, which also carries a
//! `GlesTexture`. This path is Vulkan-only, and there the compositor imports the
//! dmabuf natively (`node.rs`'s `Background3D` arm hands `e.dmabuf` to
//! `PreImported` and never reads the GLES view). Dropping that import is what lets
//! the worker own its buffers outright: the GLES import is the one step that
//! needs `&mut GlesRenderer`, which only exists on the compositor thread.
//!
//! ARGB8888 with an EXPLICIT, negotiated modifier — the same guarantee the
//! inline surfaces get, against a different intersection.
//!
//! Inline slots intersect `gles ∩ wgpu` because they build a `GlesTexture` per
//! slot whatever renderer is compositing. These slots build none: the compositor
//! imports the dmabuf natively, so the constraint is what the COMPOSITING
//! renderer can import (published once at startup by the kernel) intersected
//! with what this wgpu device can. Using the GLES set here would both
//! over-constrain — rejecting modifiers Vulkan takes and GLES does not — and
//! under-constrain, by never checking the renderer that actually samples.
//!
//! An empty intersection falls back to the implicit gbm path, byte-identical to
//! before. That matters: an implicit allocation can report modifier INVALID,
//! which is exactly what the wgpu import refuses on AMD, and it is the case the
//! negotiated path was added for.

pub mod slot;
pub use slot::*;
