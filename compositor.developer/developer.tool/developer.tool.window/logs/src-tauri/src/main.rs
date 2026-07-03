// y5 developer log viewer — Tauri backend.
//
// Connects to the compositor's log gRPC server over its unix socket, streams structured
// records, and forwards each one to the React frontend as a Tauri `log` event. Reconnects
// automatically while the compositor is down.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio_stream::StreamExt;

mod proto {
    tonic::include_proto!("y5.developer.logs");
}

mod store;

// Filter presets and log dumps persisted under ~/.config/y5.compositor.developer/.
#[tauri::command]
fn list_presets() -> Result<Vec<String>, String> {
    store::list("presets")
}
#[tauri::command]
fn save_preset(name: String, data: String) -> Result<(), String> {
    store::save("presets", &name, &data)
}
#[tauri::command]
fn load_preset(name: String) -> Result<String, String> {
    store::read("presets", &name)
}
#[tauri::command]
fn delete_preset(name: String) -> Result<(), String> {
    store::remove("presets", &name)
}
#[tauri::command]
fn list_dumps() -> Result<Vec<String>, String> {
    store::list("dumps")
}
#[tauri::command]
fn save_dump(name: String, data: String) -> Result<(), String> {
    store::save("dumps", &name, &data)
}
#[tauri::command]
fn load_dump(name: String) -> Result<String, String> {
    store::read("dumps", &name)
}
#[tauri::command]
fn delete_dump(name: String) -> Result<(), String> {
    store::remove("dumps", &name)
}

/// Same socket the compositor's `process.instance` binds.
const SOCKET: &str = "/tmp/y5-compositor-logs.sock";

#[derive(Clone, Serialize)]
struct EnvFlag {
    key: String,
    value: String,
}

/// Serde mirror of the proto `CompositorStats` for the Statistics tab.
#[derive(Clone, Serialize)]
struct CompositorStats {
    renderer: String,
    renderer_init_ok: bool,
    sync_mode: String,
    output_name: String,
    mode: String,
    vrr_supported: bool,
    vrr_enabled: bool,
    hdr_enabled: bool,
    frames_total: u64,
    vblanks_total: u64,
    fps: f32,
    vblank_rate: f32,
    fence_synchronous: u64,
    fence_kms_infence: u64,
    fence_fallback: u64,
    uptime_secs: f64,
    env_flags: Vec<EnvFlag>,
    hdr_capable: bool,
    hdr_transfer: String,
    hdr_max_luminance: f32,
    hdr_bt2020: bool,
    color_format: String,
    device_formats: Vec<DeviceFormat>,
}

/// Serde mirror of the proto `DeviceFormat` (GPU formats panel).
#[derive(Clone, Serialize)]
struct DeviceFormat {
    kind: String,
    fourcc: String,
    modifier: String,
    class: String,
    plane_count: u32,
    multiplane: bool,
}

/// Connect to the compositor and pull a one-shot diagnostics snapshot.
#[tauri::command]
async fn get_compositor_stats() -> Result<CompositorStats, String> {
    let channel = tonic::transport::Endpoint::try_from("http://127.0.0.1:50051")
        .map_err(|e| e.to_string())?
        .connect_with_connector(tower::service_fn(|_| async {
            let stream = tokio::net::UnixStream::connect(SOCKET).await?;
            Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
        }))
        .await
        .map_err(|e| format!("compositor not connected: {e}"))?;
    let mut client = proto::log_stream_client::LogStreamClient::new(channel);
    let s = client
        .get_stats(proto::StatsRequest {})
        .await
        .map_err(|e| e.to_string())?
        .into_inner();
    Ok(CompositorStats {
        renderer: s.renderer,
        renderer_init_ok: s.renderer_init_ok,
        sync_mode: s.sync_mode,
        output_name: s.output_name,
        mode: s.mode,
        vrr_supported: s.vrr_supported,
        vrr_enabled: s.vrr_enabled,
        hdr_enabled: s.hdr_enabled,
        frames_total: s.frames_total,
        vblanks_total: s.vblanks_total,
        fps: s.fps,
        vblank_rate: s.vblank_rate,
        fence_synchronous: s.fence_synchronous,
        fence_kms_infence: s.fence_kms_infence,
        fence_fallback: s.fence_fallback,
        uptime_secs: s.uptime_secs,
        env_flags: s
            .env_flags
            .into_iter()
            .map(|f| EnvFlag {
                key: f.key,
                value: f.value,
            })
            .collect(),
        hdr_capable: s.hdr_capable,
        hdr_transfer: s.hdr_transfer,
        hdr_max_luminance: s.hdr_max_luminance,
        hdr_bt2020: s.hdr_bt2020,
        color_format: s.color_format,
        device_formats: s
            .device_formats
            .into_iter()
            .map(|d| DeviceFormat {
                kind: d.kind,
                fourcc: d.fourcc,
                modifier: d.modifier,
                class: d.class,
                plane_count: d.plane_count,
                multiplane: d.multiplane,
            })
            .collect(),
    })
}

/// Live HDR tuning pushed from the frontend sliders (mirrors proto `HdrParams`).
#[derive(serde::Deserialize)]
struct HdrParams {
    enabled: f32,
    sdr_white_nits: f32,
    max_nits: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    gamut: f32,
    tone_map: f32,
    transfer: f32,
    gamma: f32,
    exposure: f32,
}

/// Push live HDR tuning to the compositor (developer tool sliders).
#[tauri::command]
async fn set_hdr_params(params: HdrParams) -> Result<(), String> {
    let channel = tonic::transport::Endpoint::try_from("http://127.0.0.1:50051")
        .map_err(|e| e.to_string())?
        .connect_with_connector(tower::service_fn(|_| async {
            let stream = tokio::net::UnixStream::connect(SOCKET).await?;
            Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
        }))
        .await
        .map_err(|e| format!("compositor not connected: {e}"))?;
    let mut client = proto::log_stream_client::LogStreamClient::new(channel);
    client
        .set_hdr_params(proto::HdrParams {
            enabled: params.enabled,
            sdr_white_nits: params.sdr_white_nits,
            max_nits: params.max_nits,
            brightness: params.brightness,
            contrast: params.contrast,
            saturation: params.saturation,
            gamut: params.gamut,
            tone_map: params.tone_map,
            transfer: params.transfer,
            gamma: params.gamma,
            exposure: params.exposure,
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Serde-friendly mirror of the proto `LogRecord`, emitted to the frontend.
#[derive(Clone, Serialize)]
struct LogRecord {
    elapsed_micros: u64,
    level: u32,
    crate_name: String,
    function: String,
    message: String,
}

fn main() {
    // webkit2gtk renders a blank window on nested / NVIDIA Wayland sessions unless its
    // DMABUF renderer is disabled. Must be set before the webview is created. Honor an
    // existing override.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_presets,
            save_preset,
            load_preset,
            delete_preset,
            list_dumps,
            save_dump,
            load_dump,
            delete_dump,
            get_compositor_stats,
            set_hdr_params,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || stream_logs(handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Dedicated thread + tokio runtime that owns the gRPC connection and reconnect loop.
fn stream_logs(app: tauri::AppHandle) {
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else {
        return;
    };
    rt.block_on(async move {
        loop {
            let _ = run_stream(&app).await;
            let _ = app.emit("status", "disconnected");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

async fn run_stream(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // tonic over a unix socket: the URI is ignored; the connector dials the socket.
    let channel = tonic::transport::Endpoint::try_from("http://127.0.0.1:50051")?
        .connect_with_connector(tower::service_fn(|_| async {
            let stream = tokio::net::UnixStream::connect(SOCKET).await?;
            Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
        }))
        .await?;

    let mut client = proto::log_stream_client::LogStreamClient::new(channel);
    let mut stream = client.stream(proto::StreamRequest {}).await?.into_inner();
    app.emit("status", "connected")?;
    eprintln!("[viewer] connected to {SOCKET}");

    while let Some(record) = stream.message().await? {
        app.emit(
            "log",
            LogRecord {
                elapsed_micros: record.elapsed_micros,
                level: record.level,
                crate_name: record.crate_name,
                function: record.function,
                message: record.message,
            },
        )?;
    }
    Ok(())
}
