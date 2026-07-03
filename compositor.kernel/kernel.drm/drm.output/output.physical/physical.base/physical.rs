//! Smithay Output + PhysicalProperties from connector info / EDID identity /
//! connector.kind orientation. (Ex wire.rs `new()` step 7.)
//! P1 default: with no readable EDID, properties match the original
//! hardcoded "Native"/"Monitor"/"Unknown" exactly.

use compositor_kernel_drm_edid_identity_base::identity::MonitorIdentity;
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::connector;
use smithay::utils::{Size, Transform};

pub fn create(info: &connector::Info, identity: &MonitorIdentity) -> Output {
    let (size_x, size_y) = info.size().unwrap_or((0, 0));
    Output::new(
        // Output NAME kept as the connector name so it is unique per output (used
        // as a friendly label / debug id). The IDENTITY that keys everything
        // (render, settings, prefs, teleport) lives in `PhysicalProperties` below
        // and already falls back to the connector when the EDID has no serial.
        format!("{:?}-{}", info.interface(), info.interface_id()),
        PhysicalProperties {
            size: Size::new(size_x as i32, size_y as i32),
            subpixel: Subpixel::Unknown,
            make: identity.make.clone().into(),
            model: identity.model.clone().into(),
            serial_number: identity.serial.clone().into(),
        },
    )
}

/// Apply the initial output state exactly as the original did (preferred mode,
/// custom 1.0 scale, origin position), plus an optional panel-orientation
/// transform from `connector.kind`.
pub fn apply_initial_state(
    output: &Output,
    mode: Mode,
    orientation: Option<Transform>,
    position: (i32, i32),
) {
    output.set_preferred(mode);
    output.change_current_state(
        Some(mode),
        orientation,
        Some(Scale::Custom {
            advertised_integer: 1,
            fractional: 1.0,
        }),
        Some(position.into()),
    );
}
