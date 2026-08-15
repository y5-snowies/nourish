//! The compositor -> worker channel, and the handle the registry drives it by.
//!
//! Everything crossing here is `Send`, which took no fighting: `BevyScene` and
//! `BevyScene::Command` are already `Send + Sync`, because the contract was
//! always command-in / texture-out. The one type that is NOT `Send` is bevy's own
//! `App` — it holds a boxed runner — and it never crosses, because [`Job::Create`]
//! carries a FACTORY rather than a built runtime. Every `App` is therefore born on
//! the worker thread and dropped there, and its `!Send` stops mattering.

pub mod base;
pub use base::*;
