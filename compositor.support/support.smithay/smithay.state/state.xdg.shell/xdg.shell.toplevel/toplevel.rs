use smithay::wayland::shell::xdg::XdgShellState;
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};

pub fn xdg_shell_state(
    dispatch: &mut Dispatch,
) -> &mut XdgShellState {
    &mut dispatch.xdg_shell.state
}
