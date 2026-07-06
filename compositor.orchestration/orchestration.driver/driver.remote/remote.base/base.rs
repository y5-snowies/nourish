use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_remote_message_state_base::state::State;
use tokio::sync::mpsc::Sender;

/// Remote driver data: the RPC state (broadcast sender + incoming buffer) lives
/// in the kernel/driver storage by token, not as an Orchestrator field.
pub static RPC: Token<State> = Token::new();
pub static RPC_MUT: TokenMut<State> = TokenMut::new(&RPC);

/// Nudge channel to the background gRPC thread: send `()` to ask it to re-check
/// its listening socket and rebind if another compositor instance replaced the
/// socket file. Pinged on session activate (VT switch back) so the daemon can
/// reconnect after a second TTY stole (and then vacated) the socket.
pub static RPC_REBIND: Token<Sender<()>> = Token::new();
