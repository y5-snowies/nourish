//! The gRPC side of the log process: a tonic server-streaming `LogStream` service on a
//! unix socket. Each new viewer first receives the buffered history, then the live stream.

pub mod serve;
pub use serve::*;
