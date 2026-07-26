// The single source of the compositor's runtime configuration. No dependency on
// the logging crate (it sits below it in the dep graph) — uses `panic!`, not `abort!`.
pub mod base;
