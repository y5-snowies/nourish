//! The `VulkanRenderer` itself and its Smithay trait implementations.
//!
//! Split across submodules so no single file is unwieldy:
//! - [`lifecycle`]: construction (`new`/`new_default`/`validate`) + the one-time
//!   device-object creators.
//! - [`pipelines`]: the per-format composite/background/HDR pipeline caches.
//! - [`submit`]: `submit_frame` (SDR + HDR record + sync) and the post-scene
//!   capture handoff.
//! - [`import`]: the `Import*` trait family (dmabuf / SHM / mem), delegating the
//!   GPU work to the `memory.*` piece-crates and reusing textures via the SHM
//!   cache.
//! - [`bind`]: `Bind<Dmabuf>` + the exportable output target + the trait
//!   surface (`RendererSuper`/`Renderer`).

mod bind;
mod state;
mod import;
mod lifecycle;
mod mipgen;
mod pipelines;
/// Recording one composite: the per-op draw and the band iteration, split out so
/// `submit` reads as the frame's structure rather than its pixels.
mod compose;
mod submit;
/// Collecting the frame's world drawables and resolving the band claim, split
/// out so the composite path in `submit` stays readable.
///
/// `pub` for `tests/claim.rs`: the claim decides whether the engine draws the
/// world band at all, and getting it wrong blanks the desktop — it is worth being
/// able to assert on directly rather than only through a device.
pub mod worldset;

pub use state::VulkanRenderer;
pub(crate) use state::{CachedTarget, OutputResources};
