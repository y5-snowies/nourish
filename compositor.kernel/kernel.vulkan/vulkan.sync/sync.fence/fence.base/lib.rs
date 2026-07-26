//! Smithay `Fence` implementations backing the Vulkan async render path's
//! `SyncPoint` — extracted from `renderer.core` into `vulkan.sync` beside the
//! other sync primitives (`sync.export`, `sync.import`, `sync.timeline`).

// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod fence;
