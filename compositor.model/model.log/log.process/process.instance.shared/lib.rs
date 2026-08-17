//! State shared between the log drain thread and the gRPC `LogStream` server: the live
//! broadcast channel and the bounded history ring.

pub mod shared;
pub use shared::*;
