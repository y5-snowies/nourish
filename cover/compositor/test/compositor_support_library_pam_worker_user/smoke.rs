// Simplest possible test: call one public function of the target crate so the coverage
// report moves off 0%. The test folder name IS the fully-qualified crate under test.
use compositor_support_library_pam_worker_user::current_username;

#[test]
fn current_username_callable() {
    // Just exercise the code path; the value depends on the environment.
    let _ = current_username();
}
