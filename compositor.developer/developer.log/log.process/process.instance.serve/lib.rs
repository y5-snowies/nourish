//! The gRPC side of the log process: a tonic server-streaming `LogStream` service on a
//! unix socket. Each new viewer first receives the buffered history, then the live stream.

use std::sync::Arc;

use compositor_developer_log_process_instance_bind::{SOCKET, bind};
use compositor_developer_log_process_instance_shared::Shared;
use compositor_developer_stats_registry_base::base as stats;
use tokio_stream::StreamExt;

/// Run the tonic `LogStream` server on the unix socket.
pub fn serve(shared: Arc<Shared>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return;
    };
    rt.block_on(async move {
        let _ = std::fs::remove_file(SOCKET);
        let Ok(listener) = tokio::net::UnixListener::bind(SOCKET) else {
            return;
        };
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
        let _ = tonic::transport::Server::builder()
            .add_service(bind::log_stream_server::LogStreamServer::new(LogStreamSvc { shared }))
            .serve_with_incoming(incoming)
            .await;
    });
}

struct LogStreamSvc {
    shared: Arc<Shared>,
}

#[tonic::async_trait]
impl bind::log_stream_server::LogStream for LogStreamSvc {
    type StreamStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<bind::LogRecord, tonic::Status>> + Send + 'static>,
    >;

    async fn stream(
        &self,
        _request: tonic::Request<bind::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamStream>, tonic::Status> {
        // Subscribe first (prefer an occasional duplicate over a lost record), then snapshot
        // history. Replay history, then the live stream. Lag errors drop silently.
        let live = tokio_stream::wrappers::BroadcastStream::new(self.shared.broadcast_tx.subscribe())
            .filter_map(|item| item.ok().map(Ok));
        let history: Vec<bind::LogRecord> = {
            let hist = self.shared.history.lock().unwrap_or_else(|e| e.into_inner());
            hist.iter().cloned().collect()
        };
        let replay = tokio_stream::iter(history.into_iter().map(Ok));
        Ok(tonic::Response::new(Box::pin(replay.chain(live))))
    }

    async fn get_stats(
        &self,
        _request: tonic::Request<bind::StatsRequest>,
    ) -> Result<tonic::Response<bind::CompositorStats>, tonic::Status> {
        let s = stats::snapshot();
        Ok(tonic::Response::new(bind::CompositorStats {
            renderer: s.renderer, renderer_init_ok: s.renderer_init_ok, sync_mode: s.sync_mode,
            output_name: s.output_name, mode: s.mode, vrr_supported: s.vrr_supported,
            vrr_enabled: s.vrr_enabled, hdr_enabled: s.hdr_enabled, frames_total: s.frames_total,
            vblanks_total: s.vblanks_total, fps: s.fps, vblank_rate: s.vblank_rate,
            fence_synchronous: s.fence_synchronous, fence_kms_infence: s.fence_kms_infence,
            fence_fallback: s.fence_fallback, uptime_secs: s.uptime_secs,
            env_flags: s.env_flags.into_iter().map(|(key, value)| bind::EnvFlag { key, value }).collect(),
            hdr_capable: s.hdr_capable, hdr_transfer: s.hdr_transfer,
            hdr_max_luminance: s.hdr_max_luminance, hdr_bt2020: s.hdr_bt2020,
            color_format: s.color_format,
            device_formats: s.device_formats.into_iter().map(|d| bind::DeviceFormat {
                kind: d.kind, fourcc: d.fourcc, modifier: d.modifier, class: d.class,
                plane_count: d.plane_count, multiplane: d.multiplane,
            }).collect(),
        }))
    }

    async fn set_hdr_params(
        &self,
        request: tonic::Request<bind::HdrParams>,
    ) -> Result<tonic::Response<bind::SetHdrParamsReply>, tonic::Status> {
        let p = request.into_inner();
        stats::set_hdr_tuning(stats::HdrTuning {
            enabled: p.enabled, sdr_white_nits: p.sdr_white_nits, max_nits: p.max_nits,
            brightness: p.brightness, contrast: p.contrast, saturation: p.saturation,
            gamut: p.gamut, tone_map: p.tone_map, transfer: p.transfer, gamma: p.gamma,
            exposure: p.exposure,
        });
        Ok(tonic::Response::new(bind::SetHdrParamsReply {}))
    }
}
