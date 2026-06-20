//! Generate libav (ffmpeg 8.x) bindings against the system headers and link the
//! ffmpeg dev libraries. Hand-rolled (not ffmpeg-next / rusty_ffmpeg) because
//! ffmpeg 8.1 is newer than those crates support, and we need full struct field
//! access (AVFormatContext.pb/.streams) + the DRM_PRIME descriptor types.

use std::env;
use std::path::PathBuf;

fn main() {
    let mut clang_args: Vec<String> = Vec::new();
    let mut include_dirs: Vec<std::path::PathBuf> = Vec::new();
    for lib in [
        "libavutil",
        "libavcodec",
        "libavformat",
        "libavfilter",
        "libswscale",
    ] {
        let l = pkg_config::Config::new()
            .probe(lib)
            .unwrap_or_else(|e| panic!("ffmpeg dev lib {lib} not found (install ffmpeg-free-devel): {e}"));
        for p in &l.include_paths {
            clang_args.push(format!("-I{}", p.display()));
            include_dirs.push(p.clone());
        }
    }

    // C accessor shim for bindgen-opaque AVFormatContext fields.
    let mut cc = cc::Build::new();
    cc.file("helpers.c");
    for d in &include_dirs {
        cc.include(d);
    }
    cc.compile("y5_vaapi_helpers");
    println!("cargo:rerun-if-changed=helpers.c");

    let bindings = bindgen::Builder::default()
        .header_contents(
            "wrapper.h",
            r#"
            #include <libavcodec/avcodec.h>
            #include <libavformat/avformat.h>
            #include <libavutil/avutil.h>
            #include <libavutil/hwcontext.h>
            #include <libavutil/hwcontext_drm.h>
            #include <libavutil/pixdesc.h>
            #include <libavutil/opt.h>
            #include <libavutil/imgutils.h>
            #include <libavfilter/avfilter.h>
            #include <libavfilter/buffersrc.h>
            #include <libavfilter/buffersink.h>
            "#,
        )
        .clang_args(&clang_args)
        // Don't carry the C header doc comments into the bindings: rustdoc would treat
        // their indented blocks (e.g. "For audio:") as doctests and fail `cargo test`.
        .generate_comments(false)
        .allowlist_function("av_.*")
        .allowlist_function("avcodec_.*")
        .allowlist_function("avformat_.*")
        .allowlist_function("avfilter_.*")
        .allowlist_function("avio_.*")
        .allowlist_type("AV.*")
        .allowlist_var("AV_.*")
        .allowlist_var("AVERROR.*")
        .allowlist_var("FF_.*")
        .allowlist_var("AVIO_.*")
        .allowlist_var("AV_BUFFERSRC_.*")
        // Un-prefixed enum constants: `AV_PIX_FMT_NV12`, not `AVPixelFormat_AV_…`.
        .prepend_enum_name(false)
        // Keep structs as real (field access needed, e.g. AVFormatContext.pb).
        // bindgen otherwise falls back to opaque `_address` for structs it can't
        // derive Debug/Default on (large array fields).
        .derive_debug(false)
        .derive_default(false)
        .layout_tests(false)
        .generate()
        .expect("bindgen failed for ffmpeg headers");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings");
}
