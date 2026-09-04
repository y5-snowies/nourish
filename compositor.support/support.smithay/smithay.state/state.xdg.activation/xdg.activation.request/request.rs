use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;
use smithay::wayland::xdg_activation::{XdgActivationToken, XdgActivationTokenData};
use std::sync::Mutex;

/// One activation token a surface has been named by.
#[derive(Clone)]
pub struct ActivationDetails {
    pub token: XdgActivationToken,
    pub token_data: XdgActivationTokenData,
}

/// EVERY token a surface has been named by, oldest first.
///
/// A list, not a slot, because the slot had to be two contradictory things. The
/// placeholder matcher wants the LAUNCH token, which for a launched window is the first
/// one it presents; the destroy path retires what it finds, so it must see the same
/// value or it evicts one token while leaving another. A first-wins slot kept the two
/// agreeing at the cost of dropping every later token in silence — and a last-wins slot
/// would lose the launch correlation instead. Keeping all of them satisfies both:
/// `token_matches` tries each, and teardown retires each.
///
/// Deduplicated: a client may re-activate with a token it already used.
#[derive(Default)]
pub struct ActivationLog(Mutex<Vec<ActivationDetails>>);

impl ActivationLog {
    pub fn entries(&self) -> Vec<ActivationDetails> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// A client asked for `surface` to be activated with `token`
/// (`xdg_activation_v1.activate`).
///
/// Recording is all this does. y5 does not follow through with the focus change — the
/// protocol explicitly allows that ("might decide not to follow through with the
/// activation if it's considered unwanted") — but the token is how a launched window is
/// correlated back to the placeholder that launched it, so it is kept either way.
pub fn request_activation(
    surface: WlSurface,
    token: XdgActivationToken,
    token_data: XdgActivationTokenData,
) {
    record(&surface, token, token_data);
}

// Pure surface-data write — does not touch `Dispatch` (so this crate stays a
// leaf, letting state.base host `impl XdgActivationHandler for Dispatch`
// without a dependency cycle — document/SMITHAY_DECOUPLING.md).
fn record(surface: &WlSurface, token: XdgActivationToken, token_data: XdgActivationTokenData) {
    with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(ActivationLog::default);
        let Some(log) = states.data_map.get::<ActivationLog>() else { return };
        let mut entries = log.0.lock().unwrap_or_else(|e| e.into_inner());
        if entries.iter().any(|e| e.token == token) {
            return;
        }
        entries.push(ActivationDetails { token, token_data });
    });
}

/// Every token `surface` has been named by, oldest first. Empty when it has spoken no
/// activation protocol at all.
pub fn activations(surface: &WlSurface) -> Vec<ActivationDetails> {
    with_states(surface, |states| {
        states.data_map.get::<ActivationLog>().map(ActivationLog::entries).unwrap_or_default()
    })
}
