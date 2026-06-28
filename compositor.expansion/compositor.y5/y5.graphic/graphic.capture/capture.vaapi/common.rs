//! Shared libav helpers for the encoders (VAAPI + NVENC): error formatting,
//! the `AVERROR` macros bindgen can't emit, the C shim for bindgen-opaque
//! `AVFormatContext` fields, and a small mp4 `Muxer` both encoders reuse.

use std::ffi::{CString, c_char, c_int};
use std::path::Path;
use std::ptr;

use crate::ffi;

// libav error macros (function-like #defines bindgen skips).
#[allow(non_snake_case)]
pub(crate) const fn AVERROR(e: c_int) -> c_int {
    -e
}
pub(crate) const EAGAIN: c_int = 11; // Linux
/// `FFERRTAG('E','O','F',' ')` = `-MKTAG('E','O','F',' ')`.
pub(crate) const AVERROR_EOF: c_int =
    -((b'E' as c_int) | ((b'O' as c_int) << 8) | ((b'F' as c_int) << 16) | ((b' ' as c_int) << 24));

pub(crate) fn averr(ret: c_int) -> String {
    let mut buf = [0 as c_char; 256];
    unsafe {
        ffi::av_strerror(ret, buf.as_mut_ptr(), buf.len());
        std::ffi::CStr::from_ptr(buf.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

// C accessor shim (helpers.c) for AVFormatContext fields bindgen renders opaque.
unsafe extern "C" {
    pub(crate) fn y5_avfmt_set_pb(ctx: *mut ffi::AVFormatContext, pb: *mut ffi::AVIOContext);
    pub(crate) fn y5_avfmt_get_pb(ctx: *mut ffi::AVFormatContext) -> *mut ffi::AVIOContext;
}

/// mp4 output: one video stream fed from an already-opened codec context.
pub(crate) struct Muxer {
    fmt_ctx: *mut ffi::AVFormatContext,
    stream: *mut ffi::AVStream,
    packet: *mut ffi::AVPacket,
    stream_index: c_int,
}

impl Muxer {
    /// `codec_ctx` must be opened already (its params are copied to the stream).
    pub(crate) unsafe fn new(
        temp: &Path,
        codec_ctx: *mut ffi::AVCodecContext,
    ) -> Result<Muxer, String> {
        let mut m = Muxer {
            fmt_ctx: ptr::null_mut(),
            stream: ptr::null_mut(),
            packet: ptr::null_mut(),
            stream_index: 0,
        };
        let path = CString::new(temp.to_string_lossy().as_bytes()).unwrap();
        let r = ffi::avformat_alloc_output_context2(
            &mut m.fmt_ctx,
            ptr::null_mut(),
            c"mp4".as_ptr(),
            path.as_ptr(),
        );
        if r < 0 || m.fmt_ctx.is_null() {
            return Err(format!("avformat_alloc_output_context2: {}", averr(r)));
        }
        m.stream = ffi::avformat_new_stream(m.fmt_ctx, ptr::null());
        if m.stream.is_null() {
            return Err("avformat_new_stream failed".into());
        }
        m.stream_index = (*m.stream).index;
        (*m.stream).time_base = (*codec_ctx).time_base;
        let r = ffi::avcodec_parameters_from_context((*m.stream).codecpar, codec_ctx);
        if r < 0 {
            return Err(format!("avcodec_parameters_from_context: {}", averr(r)));
        }
        let mut pb: *mut ffi::AVIOContext = ptr::null_mut();
        let r = ffi::avio_open(&mut pb, path.as_ptr(), ffi::AVIO_FLAG_WRITE as c_int);
        if r < 0 {
            return Err(format!("avio_open: {}", averr(r)));
        }
        y5_avfmt_set_pb(m.fmt_ctx, pb);
        let r = ffi::avformat_write_header(m.fmt_ctx, ptr::null_mut());
        if r < 0 {
            return Err(format!("avformat_write_header: {}", averr(r)));
        }
        m.packet = ffi::av_packet_alloc();
        if m.packet.is_null() {
            return Err("av_packet_alloc failed".into());
        }
        Ok(m)
    }

    /// Send one frame (or NULL to flush) and write any emitted packets.
    pub(crate) unsafe fn pump(&mut self, codec_ctx: *mut ffi::AVCodecContext, frame: *mut ffi::AVFrame) {
        let r = ffi::avcodec_send_frame(codec_ctx, frame);
        if r < 0 && r != AVERROR(EAGAIN) {
            warn!("encode: send_frame: {}", averr(r));
            return;
        }
        loop {
            let r = ffi::avcodec_receive_packet(codec_ctx, self.packet);
            if r == AVERROR(EAGAIN) || r == AVERROR_EOF {
                break;
            }
            if r < 0 {
                warn!("encode: receive_packet: {}", averr(r));
                break;
            }
            (*self.packet).stream_index = self.stream_index;
            ffi::av_packet_rescale_ts(self.packet, (*codec_ctx).time_base, (*self.stream).time_base);
            let r = ffi::av_interleaved_write_frame(self.fmt_ctx, self.packet);
            if r < 0 {
                warn!("encode: write_frame: {}", averr(r));
            }
            ffi::av_packet_unref(self.packet);
        }
    }

    /// Flush the encoder and finalize the mp4.
    pub(crate) unsafe fn finish(&mut self, codec_ctx: *mut ffi::AVCodecContext) {
        self.pump(codec_ctx, ptr::null_mut());
        ffi::av_write_trailer(self.fmt_ctx);
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        unsafe {
            if !self.packet.is_null() {
                ffi::av_packet_free(&mut self.packet);
            }
            if !self.fmt_ctx.is_null() {
                let mut pb = y5_avfmt_get_pb(self.fmt_ctx);
                if !pb.is_null() {
                    ffi::avio_closep(&mut pb);
                    y5_avfmt_set_pb(self.fmt_ctx, ptr::null_mut());
                }
                ffi::avformat_free_context(self.fmt_ctx);
                self.fmt_ctx = ptr::null_mut();
            }
        }
    }
}
