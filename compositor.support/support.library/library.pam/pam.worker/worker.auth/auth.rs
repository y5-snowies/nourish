pub use compositor_support_library_pam_worker_zerostr::ZeroString;

/// Sent from the main thread to the worker.
pub struct AuthRequest {
    pub password: ZeroString,
}

/// Sent from the worker back to the main thread.
#[derive(Debug, Clone)]
pub enum AuthResponse {
    /// User is authenticated and the account is valid.
    Success,
    /// Wrong password, account locked, etc.
    Failure(String),
    /// PAM itself failed (service file missing, module crash, etc.).
    Error(String),
}

#[derive(Debug)]
pub enum SubmitError {
    /// Another attempt is already queued or being processed.
    Busy,
    /// The worker thread has exited. Respawn the worker.
    WorkerDead,
}
