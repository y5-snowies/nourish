pub mod message;

// The generated protobuf bindings live in `message` (a lib.rs holds no
// definitions); the glob carries them, and the service traits, back to the
// crate root where callers have always found them.
pub use message::*;
