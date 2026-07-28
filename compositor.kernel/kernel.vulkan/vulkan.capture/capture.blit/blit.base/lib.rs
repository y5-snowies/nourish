//! Post-scene capture (native Vulkan path): copy the composed scene into the
//! capture registry's entry dmabufs via `vkCmdBlitImage`, extracted from
//! `renderer.core`.
//!
//! The [`CaptureCache`] caches the entry dmabufs imported as TRANSFER_DST
//! images. Unlike the previous renderer-internal map (keyed by `(raw_fd, w, h)`
//! and never evicted — a VRAM leak on output/scale reconfigure, plus an
//! fd-reuse aliasing hazard), this cache resynchronises to the *current* target
//! set every frame: if the set changed (resize, output reconfigure, lock/unlock)
//! it frees the previous images and re-imports, so stale full-screen images are
//! never retained.

// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod blit;
