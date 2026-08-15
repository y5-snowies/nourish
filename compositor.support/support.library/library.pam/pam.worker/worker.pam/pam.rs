use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, TryRecvError, TrySendError};
use std::thread;
use smithay::reexports::calloop::ping::{make_ping, Ping, PingSource};
use compositor_support_library_pam_worker_auth::{AuthRequest, AuthResponse, SubmitError};
use compositor_support_library_pam_worker_zerostr::ZeroString;

pub struct PamWorker {
    request_tx: std::sync::mpsc::SyncSender<AuthRequest>,
    response_rx: Receiver<AuthResponse>,
    ping_source: Option<PingSource>,
    _worker: thread::JoinHandle<()>,
}

impl PamWorker {
    pub fn spawn(service: &'static str, username: String) -> std::io::Result<Self> {
        let (request_tx, request_rx) = sync_channel::<AuthRequest>(1);
        let (response_tx, response_rx) = channel::<AuthResponse>();
        let (ping, ping_source) = make_ping().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("calloop ping creation failed: {e}"),
            )
        })?;

        let worker = thread::Builder::new()
            .name(format!("pam-worker-{service}"))
            .spawn(move || worker_loop(service, username, request_rx, response_tx, ping))?;

        Ok(Self {
            request_tx,
            response_rx,
            ping_source: Some(ping_source),
            _worker: worker,
        })
    }

    pub fn take_ping_source(&mut self) -> Option<PingSource> {
        self.ping_source.take()
    }

    pub fn try_submit(&self, password: ZeroString) -> Result<(), SubmitError> {
        let req = AuthRequest { password };
        match self.request_tx.try_send(req) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(req)) => { drop(req); Err(SubmitError::Busy) }
            Err(TrySendError::Disconnected(req)) => { drop(req); Err(SubmitError::WorkerDead) }
        }
    }

    pub fn drain_responses(&self) -> Vec<AuthResponse> {
        let mut out = Vec::new();
        loop {
            match self.response_rx.try_recv() {
                Ok(r) => out.push(r),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        out
    }
}

fn worker_loop(
    service: &'static str,
    username: String,
    request_rx: Receiver<AuthRequest>,
    response_tx: Sender<AuthResponse>,
    ping: Ping,
) {
    while let Ok(request) = request_rx.recv() {
        let response = run_single_attempt(service, &username, &request.password);
        drop(request);
        if response_tx.send(response).is_err() { break; }
        ping.ping();
    }
}

fn run_single_attempt(service: &str, username: &str, password: &str) -> AuthResponse {
    use pam_client::conv_mock::Conversation;
    use pam_client::{Context, Flag};

    let conv = Conversation::with_credentials(username, password);
    let mut ctx = match Context::new(service, Some(username), conv) {
        Ok(ctx) => ctx,
        Err(e) => return AuthResponse::Error(format!("pam_start({service}): {e}")),
    };
    if let Err(e) = ctx.authenticate(Flag::NONE) {
        return AuthResponse::Failure(format!("Authentication failed: {e}"));
    }
    if let Err(e) = ctx.acct_mgmt(Flag::NONE) {
        return AuthResponse::Failure(format!("Account check failed: {e}"));
    }
    AuthResponse::Success
}
