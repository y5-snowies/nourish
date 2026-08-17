//! Logging frontend global state: the fan-in buffer sender, the application start
//! instant, the runtime level mask — plus the backing `abort` function. Split out of
//! `instance.record` (the macro crate).

pub mod channel;
pub use channel::*;
