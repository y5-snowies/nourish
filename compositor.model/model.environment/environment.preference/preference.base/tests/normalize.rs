//! `outputs_default_mode` refresh normalization: implausibly-small refresh values
//! (e.g. a hand-edited `60` meaning 60 Hz, or nonsense) marshal up to 30 Hz; a
//! plausible value is left untouched; absence is a no-op.
use compositor_model_environment_preference_base::base::{normalize, DefaultMode, Preference};

fn with_default_mode(refresh_mhz: u32) -> Preference {
    Preference {
        outputs_default_mode: Some(DefaultMode { width: 1920, height: 1080, refresh_mhz }),
        ..Preference::default()
    }
}

#[test]
fn implausible_refresh_marshals_to_30hz() {
    assert_eq!(normalize(with_default_mode(60)).outputs_default_mode.unwrap().refresh_mhz, 30_000);
    assert_eq!(normalize(with_default_mode(0)).outputs_default_mode.unwrap().refresh_mhz, 30_000);
}

#[test]
fn plausible_refresh_is_preserved() {
    assert_eq!(normalize(with_default_mode(60_000)).outputs_default_mode.unwrap().refresh_mhz, 60_000);
    assert_eq!(normalize(with_default_mode(144_000)).outputs_default_mode.unwrap().refresh_mhz, 144_000);
}

#[test]
fn absent_default_mode_is_noop() {
    assert!(normalize(Preference::default()).outputs_default_mode.is_none());
}
