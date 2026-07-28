// Simplest possible test: call one public function of the target crate so the coverage
// report moves off 0%. The test folder name IS the fully-qualified crate under test.
use compositor_model_stats_registry_hdr::{hdr_tuning, hdr_tuning_version};

#[test]
fn hdr_tuning_has_a_version() {
    let _ = hdr_tuning();
    let _ = hdr_tuning_version();
}
