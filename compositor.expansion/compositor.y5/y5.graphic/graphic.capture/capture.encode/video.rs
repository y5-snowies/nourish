//! Video encoding via an ffmpeg subprocess.
//!
//! We pipe raw BGRA frames to `ffmpeg`'s stdin and let it mux to mp4 (codec
//! auto-selected by the container — H.264 where available, else mpeg4). A
//! dedicated thread owns stdin so the render path never blocks on the pipe.
//! Output goes to a temp file; the caller moves it to the final destination on
//! Save (or deletes it on Discard).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Stdio};

use compositor_support_library_process_child_hygiene::hygiene::command;
use compositor_support_library_process_child_spawn::spawn as child_spawn;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

pub struct VideoEncoder {
    tx: Option<Sender<Vec<u8>>>,
    join: Option<JoinHandle<()>>,
    child: Option<Child>,
    temp: PathBuf,
    width: u32,
    height: u32,
}

impl VideoEncoder {
    /// Spawn ffmpeg encoding `width`×`height` BGRA frames at `fps` to a temp
    /// mp4. Dimensions are forced even (yuv420p requires it). Returns `None`
    /// if ffmpeg can't be spawned (treated as "video unavailable").
    pub fn start(width: u32, height: u32, fps: u32) -> Option<Self> {
        let w = width & !1;
        let h = height & !1;
        if w == 0 || h == 0 {
            return None;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp = std::env::temp_dir().join(format!("y5-capture-{nanos}.mp4"));

        let mut cmd = command("ffmpeg");
        cmd.args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &fps.max(1).to_string(),
            "-i",
            "-",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&temp)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let mut child = child_spawn::spawn(&mut cmd)
            .map_err(|e| warn!("ffmpeg spawn failed (video disabled): {e}"))
            .ok()?;

        let mut stdin = child.stdin.take()?;
        let (tx, rx) = channel::<Vec<u8>>();
        let join = std::thread::spawn(move || {
            while let Ok(buf) = rx.recv() {
                if stdin.write_all(&buf).is_err() {
                    break;
                }
            }
            // Dropping `stdin` here signals EOF so ffmpeg flushes and exits.
        });

        Some(Self {
            tx: Some(tx),
            join: Some(join),
            child: Some(child),
            temp,
            width: w,
            height: h,
        })
    }

    /// The fixed encoder dimensions (even). Frames must be fit to this size.
    pub fn dims(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Queue one BGRA frame (must be exactly `width*height*4` bytes).
    pub fn push(&self, bgra: Vec<u8>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(bgra);
        }
    }

    /// Stop the stream, wait for ffmpeg to flush, and return the temp mp4 path
    /// (or `None` if ffmpeg exited non-zero / produced nothing).
    pub fn finish(mut self) -> Option<PathBuf> {
        self.tx = None; // close the channel → writer thread ends → stdin EOF
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        let status = self.child.take().and_then(|mut c| c.wait().ok());
        match status {
            Some(s) if s.success() => Some(std::mem::take(&mut self.temp)),
            other => {
                warn!("ffmpeg exited unsuccessfully: {other:?}");
                let _ = std::fs::remove_file(&self.temp);
                None
            }
        }
    }

    /// Drop the encoder and delete the temp file (discard path).
    pub fn discard(mut self) {
        self.tx = None;
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&self.temp);
    }
}
