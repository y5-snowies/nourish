//! The trait every Bevy scene in this system implements.
//!
//! Each scene defines a `Command` type and methods to build the world and
//! apply commands.
//!
//! Texture inputs are passed to the scene's constructor as plain
//! `Handle<Image>` values, the same way any other field would be. The
//! caller obtains these handles via `BevyRegistry::import_dmabuf(...)` and
//! stores them in the scene struct. To swap a texture later, dispatch a
//! command that carries a new `Handle<Image>` and mutate the relevant
//! resource/asset inside `apply_command`.

pub mod base;
pub use base::*;
