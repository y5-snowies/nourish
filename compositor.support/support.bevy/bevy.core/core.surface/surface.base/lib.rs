//! `BevySurface`: the ring of dmabufs one Bevy instance renders through.
//!
//! Each entry is a [`Slot`] — one dmabuf imported as both a `wgpu::Texture`
//! (Bevy's render attachment) and a `GlesTexture` (what the compositor samples).
//! The engine draws into [`target`](BevySurface::target); the compositor is only
//! ever shown [`published`](BevySurface::published), a buffer whose write has
//! already completed. Ring depth is the live `Surfaces` setting: one slot is the
//! disabled path and behaves exactly as the single-buffer surface did.

pub mod base;
pub use base::*;
