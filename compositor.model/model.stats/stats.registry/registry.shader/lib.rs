//! Process-global default background-shader selection, seeded once from the user
//! preference at startup. Lives behind an `RwLock` (not the `Meta` mutex) so the
//! per-world read path is cheap and a settings reload can reseed it.
//!
//! This is the seam that lets `background.two`'s system — which has no world id
//! or preference access in `update()` — resolve the default without plumbing the
//! preference through the system layer. A world's own record may still override
//! it; resolution is `world_override.or_else(background_shader_default)`.
//!
//! NOTHING off-thread lives here any more. The publish/wake handshake moved to
//! `kernel.graphic`'s `bridge.publish/publish.wake`, next to the ring the
//! producers publish through; the background triple-buffer settings moved into
//! `environment.background`'s own `base`, which is where the UI setting already
//! kept its live copy. Both sat here only because this was a crate everybody
//! could already reach — not a reason for a diagnostics registry to own them.

pub mod shader;
pub use shader::*;
