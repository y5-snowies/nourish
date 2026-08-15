pub mod message;

// The generated protobuf bindings live in `message` (a lib.rs holds no
// definitions); re-exported here so `<crate>::bind::…` is unchanged.
pub use message::bind;
