//! The log drain thread: consumes the global fan-in buffer, prints each record
//! dmesg-style (elapsed-since-start), keeps the bounded history ring, and fans records
//! out over the tokio broadcast to every connected gRPC viewer.

pub mod drain;
pub use drain::*;
