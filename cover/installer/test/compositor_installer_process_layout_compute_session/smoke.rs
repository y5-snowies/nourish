// Simplest possible test: call one public function of the target crate so the coverage
// report moves off 0%. The test folder name IS the fully-qualified crate under test.
use compositor_installer_process_layout_compute_session::shutdown_target;

#[test]
fn shutdown_target_is_nonempty() {
    let t = shutdown_target();
    assert!(!t.is_empty(), "shutdown target should not be empty");
}
