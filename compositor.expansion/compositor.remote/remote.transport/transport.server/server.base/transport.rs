use smithay::reexports::calloop;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::channel::Sender;
use std::fs::remove_file;
use std::os::unix::fs::MetadataExt;
use tokio::net::UnixListener;
use tokio::sync::mpsc::Receiver;
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;
use compositor_orchestration_core_state_base::Loop;

const SOCKET_PATH: &str = "/tmp/y5-compositor-rpc.sock";

pub struct Transport {
    pub broadcast_transmit:
        tokio::sync::broadcast::Sender<compositor_remote_message_server_base::message::Message>,
    /// Nudge the gRPC thread to re-check its socket and rebind if a second
    /// compositor (another TTY) replaced the file. Pinged on session activate
    /// via non-blocking `try_send` (capacity 1 — extra pings just drop).
    pub rebind_signal: tokio::sync::mpsc::Sender<()>,
}

pub fn create(loop_handle: LoopHandle<Loop>) -> Transport {
    let (calloop_tx, calloop_rx) =
        calloop::channel::channel::<compositor_remote_message_client_base::message::Message>();
    // Seems that broadcast is never handled (Calloop -> gRPC, capacity 100)
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<
        compositor_remote_message_server_base::message::Message,
    >(100);
    // "Please re-check the socket" nudges (Calloop -> gRPC, capacity 1).
    let (rebind_tx, rebind_rx) = tokio::sync::mpsc::channel::<()>(1);

    // Insert the Calloop receiver: populates incoming_buffer in the loop.
    loop_handle
        .insert_source(calloop_rx, |event, _meta, state: &mut Loop| {
            if let calloop::channel::Event::Msg(cmd) = event {
                state.inner.kernel.get_mut(&compositor_orchestration_driver_remote_base::base::RPC_MUT).incoming_buffer.push(cmd);
            }
        })
        .unwrap();

    background(calloop_tx, rebind_rx);
    return Transport { broadcast_transmit: broadcast_tx, rebind_signal: rebind_tx };
}

fn background(
    calloop_tx: Sender<compositor_remote_message_client_base::message::Message>,
    mut rebind_rx: Receiver<()>,
) {
    std::thread::spawn(move || {
        // current_thread: drive all async on THIS thread, no worker pool.
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            // Create the server ONCE — no reconstruction loop. Every rebind is a
            // channel nudge (arrives only on session activate), so it can't spin.
            let mut server = serve(calloop_tx.clone());
            while rebind_rx.recv().await.is_some() {
                // Leave it alone while the path still points at our own socket.
                let ours = matches!(&server, Some((ino, _))
                    if Some(*ino) == std::fs::metadata(SOCKET_PATH).map(|m| m.ino()).ok());
                if !ours {
                    if let Some((_, stop)) = server.take() { let _ = stop.send(()); }
                    server = serve(calloop_tx.clone());
                }
            }
        });
    });
}

// Bind the socket and spawn a fresh gRPC server task; returns the bound inode
// (so a nudge can tell whether the path was replaced) plus a shutdown handle
// that retires this task before the next rebind.
fn serve(
    calloop_tx: Sender<compositor_remote_message_client_base::message::Message>,
) -> Option<(u64, oneshot::Sender<()>)> {
    let _ = remove_file(SOCKET_PATH);
    let uds = match UnixListener::bind(SOCKET_PATH) {
        Ok(uds) => uds,
        Err(e) => { error!("Failed to bind gRPC socket {}: {}", SOCKET_PATH, e); return None; }
    };
    let ino = std::fs::metadata(SOCKET_PATH).map(|m| m.ino()).ok()?;
    let (stop_tx, stop_rx) = oneshot::channel();
    let stream = UnixListenerStream::new(uds);
    let service = compositor_remote_message_client_base::message::MessageClientReceiver { calloop_tx };
    info!("Starting gRPC server on {}", SOCKET_PATH);
    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(compositor_remote_message_client_base::bind::navigator::navigator_server::NavigatorServer::new(service.clone()))
            .add_service(compositor_remote_message_client_base::bind::debug::debug_server::DebugServer::new(service.clone()))
            .add_service(compositor_remote_message_client_base::bind::selection::selection_server::SelectionServer::new(service.clone()))
            .serve_with_incoming_shutdown(stream, async { let _ = stop_rx.await; })
            .await;
    });
    Some((ino, stop_tx))
}
