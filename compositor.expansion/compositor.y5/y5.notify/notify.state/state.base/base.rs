//! The notification queue: the kernel-hosted `NotifySystem`'s slot, and the
//! channel that feeds it.
//!
//! A notification is raised by whatever noticed something (a placeholder in
//! world A adopting a window while world B is on screen, a keybind, ...). None
//! of those belong to the world that happens to be resident, so the slot lives
//! in the KERNEL host's storage, not in any world's, and a raise is an event on
//! the host's router: `announce(state.inner.kernel_channels(), msg)`.
//!
//! FIFO: a second notification raised while the first is up waits its turn
//! rather than replacing it. Only the owning system mutates the queue (through
//! its buffer); nothing here knows how long a message stays on screen.

use std::collections::VecDeque;

use compositor_support_system_channel_router_base::base::ChannelRouter;
use compositor_support_system_channel_token_base::y5_channel;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};

#[derive(Default)]
pub struct NotifyState {
    pub queue: VecDeque<String>,
}

/// The kernel host's notification slot. The write token is public because the
/// owning system is another crate, exactly as `BG_TWO_MUT` is for `TwoSystem`.
pub static NOTIFY: Token<NotifyState> = Token::new();
pub static NOTIFY_MUT: TokenMut<NotifyState> = TokenMut::new(&NOTIFY);

// This crate is the single sender; `NotifySystem` is the receiver.
y5_channel!(pub NOTIFY_REQUEST, NOTIFY_REQUEST_TX: String);

/// Queue a notification for the user, on the KERNEL host's router. The
/// message lands in the slot when the host next drains (the coming frame).
pub fn announce(channels: &mut ChannelRouter, message: impl Into<String>) {
    channels.send(&NOTIFY_REQUEST_TX, message.into());
}
