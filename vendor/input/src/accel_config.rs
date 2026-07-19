//! Libinput AccelConfig

use std::fmt::Debug;

use crate::{ffi, AccelProfile, AccelType, AsRaw, DeviceConfigError, DeviceConfigResult};

/// A handle for configuration pointer acceleration.
///
/// To configure pointer acceleration, first create a config of a desired
/// acceleration profile with [`AccelConfig::new`], then configure
/// the profile-specific acceleration properties.
#[derive(PartialEq, Eq, Hash)]
pub struct AccelConfig(*mut ffi::libinput_config_accel);

impl Debug for AccelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AccelConfig @{:p}", self.as_raw())
    }
}

impl AccelConfig {
    /// Create an acceleration configuration of a given profile.
    pub fn new(profile: AccelProfile) -> Option<Self> {
        let config = unsafe {
            ffi::libinput_config_accel_create(match profile {
                AccelProfile::Flat => {
                    ffi::libinput_config_accel_profile_LIBINPUT_CONFIG_ACCEL_PROFILE_FLAT
                }
                AccelProfile::Adaptive => {
                    ffi::libinput_config_accel_profile_LIBINPUT_CONFIG_ACCEL_PROFILE_ADAPTIVE
                }
                AccelProfile::Custom => {
                    ffi::libinput_config_accel_profile_LIBINPUT_CONFIG_ACCEL_PROFILE_CUSTOM
                }
            })
        };

        if config.is_null() {
            None
        } else {
            Some(Self(config))
        }
    }

    /// Defines the acceleration function for a given movement type in an
    /// acceleration configuration with the profile [`AccelProfile::Custom`].
    ///
    /// Movement types are specific to each device, see [`AccelType`].
    ///
    /// Each custom acceleration function is defined by n points spaced
    /// uniformly along the x-axis starting from 0 and continuing in a constant
    /// step size. There by the function is defined by the following points:
    /// `(0 * step, f[0]), (1 * step, f[1]), …, ((n - 1) * step, f[n - 1])`.
    /// The x-axis represents the device-speed in device units per millisecond.
    /// The y-axis represents the pointer-speed.
    ///
    /// It is up to the user to define those values in accordance with device DPI and screen DPI.
    pub fn set_points(
        &self,
        accel_type: AccelType,
        step: f64,
        points: &[f64],
    ) -> DeviceConfigResult {
        match unsafe {
            ffi::libinput_config_accel_set_points(
                self.0,
                match accel_type {
                    AccelType::Fallback => {
                        ffi::libinput_config_accel_type_LIBINPUT_ACCEL_TYPE_FALLBACK
                    }
                    AccelType::Motion => ffi::libinput_config_accel_type_LIBINPUT_ACCEL_TYPE_MOTION,
                    AccelType::Scroll => ffi::libinput_config_accel_type_LIBINPUT_ACCEL_TYPE_SCROLL,
                },
                step,
                points.len(),
                points.as_ptr() as _,
            )
        } {
            ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_SUCCESS => Ok(()),
            ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_UNSUPPORTED => {
                Err(DeviceConfigError::Unsupported)
            }
            ffi::libinput_config_status_LIBINPUT_CONFIG_STATUS_INVALID => {
                Err(DeviceConfigError::Invalid)
            }
            _ => panic!("libinput returned invalid 'libinput_config_status'"),
        }
    }
}

impl AsRaw<ffi::libinput_config_accel> for AccelConfig {
    fn as_raw(&self) -> *const ffi::libinput_config_accel {
        self.0
    }
}

impl Drop for AccelConfig {
    fn drop(&mut self) {
        unsafe { ffi::libinput_config_accel_destroy(self.0) };
    }
}
