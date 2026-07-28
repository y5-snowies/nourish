//! Shared protobuf/gRPC bindings for the log stream (`protocol/logs.proto`) plus the
//! unix-socket path — shared by the drain/serve runtime crates and their façade.

/// Generated protobuf/gRPC bindings for `protocol/logs.proto`.
pub mod bind {
    tonic::include_proto!("y5.developer.logs");
}

/// Unix socket the viewer connects to.
pub const SOCKET: &str = "/tmp/y5-compositor-logs.sock";
