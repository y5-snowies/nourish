//! Sphere mesh generator.
//!
//! Stores positions and normals as sphere coordinates. The flat-plane
//! position is computed in the vertex shader from each vertex's UV when
//! the `flatness` morph parameter is > 0.

pub mod mesh;
pub use mesh::*;
