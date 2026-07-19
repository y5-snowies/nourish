//! Pad-button routing, called from the native libinput pad handler (the pad
//! `SpecialEvent` is non-generic, so it lives in the kernel closure). Resolves the
//! user's per-button binding: an unbound button (`Passthrough`) forwards the native
//! `zwp_tablet_pad_v2` button to the focused client; a bound one runs its action
//! (toggle hand mode / open touch menu / key combo / click) and is NOT forwarded.

use compositor_developer_environment_preference_base::base::PenAction;
use compositor_orchestration_core_state_base::Loop;
use crate::tablet::action;

/// Handle one pad-button edge for device `key`, button index `button`.
pub fn button(_loop: &mut Loop, key: &str, button: u32, pressed: bool, time: u32) {
    let bound = _loop.inner.preference.pen.pad_action(key, button);
    match bound {
        PenAction::Passthrough => {
            _loop.state.tablet.pad_button(key, button, pressed, time);
        }
        other => action::execute(_loop, &other, pressed, time),
    }
}
