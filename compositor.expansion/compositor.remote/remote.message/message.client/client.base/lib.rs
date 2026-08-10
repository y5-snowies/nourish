// reexport the generated bindings
pub mod bind {
    pub mod navigator {
        tonic::include_proto!("y5.compositor.rpc.protocol.client.navigator");
    }
    pub mod debug {
        tonic::include_proto!("y5.compositor.rpc.protocol.client.debug");
    }
    pub mod selection {
        tonic::include_proto!("y5.compositor.rpc.protocol.client.selection");
    }
    pub mod shader {
        tonic::include_proto!("y5.compositor.rpc.protocol.client.shader");
    }
}

pub mod message;


pub use message::*;
