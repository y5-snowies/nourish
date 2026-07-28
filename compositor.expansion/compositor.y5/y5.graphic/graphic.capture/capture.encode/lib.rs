//! Capture encoding: GPU→CPU readback, PNG image save, mp4 video encode (via
//! an ffmpeg subprocess), default save paths, and the XDG portal "Save As"
//! file dialog.
//!
//! Saving was deferred in the first capture cut; this is where it lands. All
//! of it is best-effort and isolated from the render path (video runs in a
//! subprocess; the portal call runs on a background thread).

// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod frame;
pub mod portal;
pub mod readback;
pub mod reencode;
pub mod save;
pub mod video;

pub use frame::Frame;
pub use readback::{AsyncReadback, readback};
pub use reencode::{
    OptimizedCodec, ReencodeJob, ReencodeStatus, ffmpeg_format, partial_path, reencode_detached,
    save_fallback,
};
pub use save::{default_path, save_png};
pub use video::VideoEncoder;
