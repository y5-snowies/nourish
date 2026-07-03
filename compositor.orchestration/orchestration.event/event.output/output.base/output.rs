//! The output-presence lifecycle event, fired exactly once per real output
//! transition by the kernel hotplug path (`display.reconcile::reconcile` →
//! `wire.plugin`). Event-driven ONLY — never polled per-frame or on a timer.

use compositor_support_system_channel_router_base::base::ChannelRouter;

/// What happened to the compositor's output(s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputChange {
    /// The last connected monitor went away — the compositor is now dark.
    WentDark,
    /// A monitor returned after being dark — an output is driving again.
    Recovered,
    /// The connected set / active output changed while still driving an output
    /// (failover to another monitor, or a non-active monitor was (un)plugged).
    Changed,
}

/// The channel token. Listeners import `event::OUTPUT_CHANGED`; the sender stays
/// crate-private so this crate is the single emitter.
pub mod event {
    use super::OutputChange;
    compositor_support_system_channel_token_base::y5_channel!(pub OUTPUT_CHANGED, OUTPUT_CHANGED_TX: OutputChange);
}

/// Announce an output change on one world's router. The caller iterates EVERY
/// world (`worlds.ids()` → `world.channels()`) so backgrounded worlds' systems
/// receive it too, not just the focused one.
pub fn broadcast(channels: &mut ChannelRouter, change: OutputChange) {
    channels.send(&event::OUTPUT_CHANGED_TX, change);
}
