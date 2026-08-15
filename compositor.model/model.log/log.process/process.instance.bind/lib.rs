//! Shared protobuf/gRPC bindings for the log stream (`protocol/logs.proto`) plus the
//! unix-socket path — shared by the drain/serve runtime crates and their façade.

mod instance;
pub use instance::*;
