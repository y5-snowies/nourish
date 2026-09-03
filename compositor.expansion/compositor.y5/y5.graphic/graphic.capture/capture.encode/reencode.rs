//! Optimized post-capture re-encode via an ffmpeg subprocess.
//!
//! The live capture writes a near-lossless hardware-encoded temp (huge, instant,
//! smooth). The *optimized* output is a **software** CRF re-encode of that temp —
//! much smaller, but software CRF can't run live (libaom/SVT-AV1/x265 don't keep
//! up at 4K), so it runs here as a background `ffmpeg` job after recording stops.
//!
//! Fully decoupled from the render path: ffmpeg runs as a subprocess, a reader
//! thread parses its `-progress` stream into a 0–1 fraction, and the capture
//! state machine polls [`ReencodeJob::poll`] (non-blocking) to drive the save
//! dialog's progress bar. If anything fails the caller falls back to saving the
//! lossless temp (so a recording is never lost).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};

use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;

/// Optimized output codec → software encoder + container. Mirrors the user's
/// reference presets (AV1 via SVT-AV1, H.265/H.264 via x265/x264). `libx264`/
/// `libx265` may be absent on a stripped ffmpeg build (then the spawn fails and
/// the caller falls back / tries the next codec).
#[derive(Clone, Copy, Debug)]
pub enum OptimizedCodec {
    Av1,
    H265,
    H264,
}

impl OptimizedCodec {
    fn encode_args(self) -> &'static [&'static str] {
        match self {
            // SVT-AV1 is far faster than libaom at similar quality (~2× realtime
            // at 4K on a modern CPU). preset 8 / crf 32 = good size/quality.
            OptimizedCodec::Av1 => &["-c:v", "libsvtav1", "-preset", "8", "-crf", "32", "-g", "120"],
            OptimizedCodec::H265 => &["-c:v", "libx265", "-preset", "medium", "-crf", "28"],
            OptimizedCodec::H264 => &["-c:v", "libx264", "-preset", "medium", "-crf", "28"],
        }
    }

    /// Container extension. AV1 → webm (its natural container); H.26x → mp4.
    pub fn ext(self) -> &'static str {
        match self {
            OptimizedCodec::Av1 => "webm",
            OptimizedCodec::H265 | OptimizedCodec::H264 => "mp4",
        }
    }
}

/// Progress/terminal state of a running re-encode.
pub enum ReencodeStatus {
    /// Still encoding; `0.0..=1.0` (best-effort — may stay 0 if duration probing
    /// failed).
    Running(f32),
    /// Finished; the optimized file is at this path.
    Done(PathBuf),
    /// ffmpeg failed / produced nothing. Caller should fall back to the temp.
    Failed,
}

/// A background ffmpeg re-encode. Poll it (non-blocking) until `Done`/`Failed`.
pub struct ReencodeJob {
    child: Option<Child>,
    join: Option<JoinHandle<()>>,
    /// Progress as `out_time_us` scaled to 0..=1_000_000 (fixed-point), shared
    /// with the reader thread.
    progress_ppm: Arc<AtomicU32>,
    output: PathBuf,
    finished: bool,
}

impl ReencodeJob {
    /// Spawn an ffmpeg re-encode of `input` → `output` with `codec`'s preset.
    /// Returns `None` if ffmpeg can't be spawned (caller falls back).
    pub fn spawn(
        input: &Path,
        output: PathBuf,
        codec: OptimizedCodec,
        cfr: bool,
        format: &str,
        render_node: &str,
    ) -> Option<ReencodeJob> {
        let total_us = probe_duration_us(input).unwrap_or(0);

        let mut cmd = command("ffmpeg");
        cmd.args(["-y", "-loglevel", "error", "-nostats", "-progress", "pipe:1"])
            .args(input_decode_args(input, render_node))
            .arg("-i")
            .arg(input)
            .args(codec.encode_args())
            .args(cfr_args(input, cfr))
            .args(["-f", format])
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = child_spawn::spawn_awaited(&mut cmd)
            .map_err(|e| warn!("reencode: ffmpeg spawn failed: {e}"))
            .ok()?;

        let stdout = child.stdout.take()?;
        let progress_ppm = Arc::new(AtomicU32::new(0));
        let shared = progress_ppm.clone();
        let join = std::thread::spawn(move || {
            // ffmpeg `-progress` emits `key=value` lines; `out_time_us=<n>`
            // marks how far in the input it has consumed.
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(v) = line.strip_prefix("out_time_us=") {
                    if total_us > 0 {
                        if let Ok(us) = v.trim().parse::<i64>() {
                            let ppm = ((us.max(0) as i128 * 1_000_000) / total_us as i128)
                                .clamp(0, 1_000_000) as u32;
                            shared.store(ppm, Ordering::Relaxed);
                        }
                    }
                }
            }
        });

        info!("reencode: started {codec:?} → {}", output.display());
        Some(ReencodeJob {
            child: Some(child),
            join: Some(join),
            progress_ppm,
            output,
            finished: false,
        })
    }

    /// Non-blocking progress / completion check. Drive this from the per-frame
    /// hook to update the save dialog's progress bar.
    pub fn poll(&mut self) -> ReencodeStatus {
        if self.finished {
            return ReencodeStatus::Failed;
        }
        let Some(child) = self.child.as_mut() else {
            return ReencodeStatus::Failed;
        };
        match child.try_wait() {
            Ok(None) => {
                let f = self.progress_ppm.load(Ordering::Relaxed) as f32 / 1_000_000.0;
                ReencodeStatus::Running(f)
            }
            Ok(Some(status)) => {
                self.finished = true;
                self.child = None;
                if let Some(j) = self.join.take() {
                    let _ = j.join();
                }
                let ok = status.success()
                    && std::fs::metadata(&self.output).map(|m| m.len() > 0).unwrap_or(false);
                if ok {
                    info!("reencode: done → {}", self.output.display());
                    ReencodeStatus::Done(std::mem::take(&mut self.output))
                } else {
                    warn!("reencode: ffmpeg failed (status={status:?})");
                    let _ = std::fs::remove_file(&self.output);
                    ReencodeStatus::Failed
                }
            }
            Err(e) => {
                warn!("reencode: try_wait error: {e}");
                self.finished = true;
                ReencodeStatus::Failed
            }
        }
    }

    /// Kill the job and delete its partial output (e.g. on Discard).
    pub fn cancel(mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        let _ = std::fs::remove_file(&self.output);
    }
}

/// In-progress filename for `target`: `<stem>.y5-encoding.tmp`. The real media
/// extension is withheld until the encode finishes (the file is renamed to
/// `target` on success), so a half-written file never looks like a playable one.
/// Because the name has no media extension, the muxer is forced with `-f`
/// ([`ffmpeg_format`]).
pub fn partial_path(target: &Path) -> PathBuf {
    let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("capture");
    target.with_file_name(format!("{stem}.y5-encoding.tmp"))
}

/// The ffmpeg muxer name to force for `target`'s extension (since the partial
/// output is written to a generic `.tmp` path ffmpeg can't infer the format from).
pub fn ffmpeg_format(target: &Path) -> &'static str {
    match target.extension().and_then(|e| e.to_str()) {
        Some("webm") => "webm",
        _ => "mp4",
    }
}

/// Auto-mode (`background_encoder="ffmpeg"`) background re-encode, fire-and-forget.
/// Encodes `temp` → a `.y5-encoding` partial next to `target`, then renames it to
/// `target` on success. On any failure, falls back to saving the lossless `temp`
/// as `target` (a recording is never lost). Returns immediately; runs on its own
/// thread, so no progress bar / state-machine polling is involved.
pub fn reencode_detached(
    temp: PathBuf,
    target: PathBuf,
    codec: OptimizedCodec,
    cfr: bool,
    render_node: String,
) {
    std::thread::spawn(move || {
        let partial = partial_path(&target);
        let fmt = ffmpeg_format(&target);
        if run_blocking(&temp, &partial, codec, cfr, fmt, &render_node)
            && std::fs::rename(&partial, &target).is_ok()
        {
            let _ = std::fs::remove_file(&temp);
            info!("reencode(auto): {} done", target.display());
        } else {
            let _ = std::fs::remove_file(&partial);
            save_fallback(&temp, &target);
            warn!("reencode(auto) failed; saved lossless → {}", target.display());
        }
    });
}

/// Move (or copy, cross-device) the lossless `temp` to `target` as the final file.
pub fn save_fallback(temp: &Path, target: &Path) {
    if std::fs::rename(temp, target).is_err() && std::fs::copy(temp, target).is_ok() {
        let _ = std::fs::remove_file(temp);
    }
}

/// Spawn ffmpeg and block until exit; `true` on success with non-empty output.
fn run_blocking(
    input: &Path,
    output: &Path,
    codec: OptimizedCodec,
    cfr: bool,
    format: &str,
    render_node: &str,
) -> bool {
    let mut cmd = command("ffmpeg");
    cmd.args(["-y", "-loglevel", "error"])
        .args(input_decode_args(input, render_node))
        .arg("-i")
        .arg(input)
        .args(codec.encode_args())
        .args(cfr_args(input, cfr))
        .args(["-f", format])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = child_spawn::status(&mut cmd);
    matches!(status, Ok(s) if s.success())
        && std::fs::metadata(output).map(|m| m.len() > 0).unwrap_or(false)
}

/// ffmpeg output options forcing constant frame rate at a snapped target rate
/// (empty when CFR isn't requested). `-fps_mode cfr -r N` re-times frames
/// (dup/drop) into a steady rate that VFR-averse editors/players accept.
fn cfr_args(input: &Path, cfr: bool) -> Vec<String> {
    if !cfr {
        return Vec::new();
    }
    let target = snap_fps(probe_avg_fps(input).unwrap_or(60.0));
    vec!["-fps_mode".into(), "cfr".into(), "-r".into(), target.to_string()]
}

/// Snap a measured average fps to the nearest standard rate when within ±5,
/// otherwise to the nearest multiple of 5 (so e.g. 57 → 60, 43 → 45).
fn snap_fps(avg: f64) -> u32 {
    const STD: [u32; 7] = [24, 25, 30, 50, 60, 90, 120];
    let (best, dist) = STD
        .iter()
        .map(|&s| (s, (avg - s as f64).abs()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    if dist <= 5.0 {
        best
    } else {
        ((avg / 5.0).round() as u32).max(1) * 5
    }
}

/// Probe a file's average frame rate via `ffprobe`. `None` if unavailable.
fn probe_avg_fps(input: &Path) -> Option<f64> {
    let mut cmd = command("ffprobe");
    cmd.args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(input);
    let out = child_spawn::output(&mut cmd).ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (n, d) = s.trim().split_once('/')?;
    let n: f64 = n.trim().parse().ok()?;
    let d: f64 = d.trim().parse().ok()?;
    if d == 0.0 { None } else { Some(n / d) }
}

/// Decode-side input args for the re-encode. Some ffmpeg builds (notably
/// Fedora's `ffmpeg-free`) ship without patent-encumbered **software** decoders
/// (H.264/HEVC), so re-encoding a hardware-H.264/HEVC capture would fail at the
/// *decode* step with "Decoder not found". When the input's codec has no software
/// decoder available, fall back to **VAAPI hardware decode** (`-hwaccel vaapi`),
/// which decodes on the GPU driver and auto-downloads to system memory for the
/// software encoder. Empty (plain software decode) otherwise — notably AV1/VP9
/// captures, whose decoders are royalty-free and present everywhere. Best-effort:
/// if VAAPI also can't decode, ffmpeg errors and the caller saves the lossless temp.
fn input_decode_args(input: &Path, render_node: &str) -> Vec<String> {
    // Only the patent-encumbered codecs (H.264/HEVC) can be missing a *software*
    // decoder on a stripped ffmpeg. AV1/VP9/etc. (and thus the NVIDIA AV1 capture
    // path) always decode in software, so skip the probe entirely — the common
    // path stays a plain software re-encode with no added work.
    let Some(codec) = probe_codec_name(input) else { return Vec::new() };
    if !matches!(codec.as_str(), "h264" | "hevc" | "h265") || software_decoder_available(&codec) {
        return Vec::new();
    }
    info!("reencode: no software '{codec}' decoder — using VAAPI hwdecode for input");
    // Pin the same render node the capture/VAAPI encoder uses, so a dual-GPU box
    // (e.g. Intel iGPU + NVIDIA) decodes on the GPU that actually owns the frames
    // rather than ffmpeg's default pick.
    vec![
        "-hwaccel".into(),
        "vaapi".into(),
        "-hwaccel_device".into(),
        render_node.to_string(),
    ]
}

/// The input stream's codec name (e.g. `h264`, `hevc`, `av1`) via `ffprobe`.
fn probe_codec_name(input: &Path) -> Option<String> {
    let mut cmd = command("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name",
        "-of",
        "default=nw=1:nk=1",
    ])
    .arg(input);
    let out = child_spawn::output(&mut cmd).ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Whether the local ffmpeg has a software decoder for `codec`. Parses
/// `ffmpeg -decoders` (each row is `flags <name> <description>`, so the decoder
/// token is surrounded by spaces). Defaults to `true` if the probe itself fails
/// (can't tell → assume software decode and let ffmpeg try).
fn software_decoder_available(codec: &str) -> bool {
    let mut cmd = command("ffmpeg");
    cmd.args(["-hide_banner", "-decoders"]);
    let Ok(out) = child_spawn::output(&mut cmd) else {
        return true;
    };
    let needle = format!(" {codec} ");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.contains(&needle))
}

/// Probe a media file's duration in microseconds via `ffprobe`. `None` if
/// unavailable (progress then stays best-effort 0).
fn probe_duration_us(input: &Path) -> Option<i64> {
    let mut cmd = command("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=nw=1:nk=1",
    ])
    .arg(input);
    let out = child_spawn::output(&mut cmd).ok()?;
    let secs: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some((secs * 1_000_000.0) as i64)
}
