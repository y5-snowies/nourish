//! LaunchPlan core: the plan type plus the synthesizer trait/registry.
pub mod plan;

// `synthesizer` is defined inside `plan` (a lib.rs holds no definitions);
// re-exported so `<crate>::synthesizer::…` is unchanged.
pub use plan::synthesizer;
