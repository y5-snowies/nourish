/// Value types used by built-in hint attributes.
pub mod values {
    /// An environment variable pair, used as the value type for the EnvOverlay
    /// attribute. Each captured env var becomes its own hint, so multiple
    /// EnvOverlay items coexist in InferredHints.
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct EnvPair {
        pub key: String,
        pub value: String,
    }

    /// A toplevel icon's pixels, decoded out of the client's `wl_shm` buffer.
    ///
    /// Straight (non-premultiplied) `RGBA8888`, `width * height * 4` bytes,
    /// square (the protocol requires it). Held behind an `Arc` because this is
    /// copied out of the client's pool exactly once and then only cloned —
    /// including into every clone of the `ApplicationData` it lands in.
    ///
    /// NOT serde: pixels are live client state, not something to persist next
    /// to a launch plan, and there is deliberately no codec registered for the
    /// attribute that carries it.
    #[derive(Debug, Clone)]
    pub struct IconPixels {
        pub width: u32,
        pub height: u32,
        pub rgba: std::sync::Arc<Vec<u8>>,
    }

    /// Two `IconPixels` are the same icon when they are the same buffer — the
    /// point of the comparison is change detection, and a byte-compare of every
    /// icon every frame is exactly what the `Arc` exists to avoid.
    impl PartialEq for IconPixels {
        fn eq(&self, other: &Self) -> bool {
            self.width == other.width
                && self.height == other.height
                && std::sync::Arc::ptr_eq(&self.rgba, &other.rgba)
        }
    }

    /// What a client declared over `xdg_toplevel_icon_v1` for one toplevel.
    ///
    /// Both halves are optional and independent: a client may name a stock icon,
    /// hand over pixel buffers, or do both. This is SURFACE state — it is read
    /// synchronously from a live window and passed into hint inference as extra
    /// context; it is deliberately not part of [`Meta`](crate::types::Meta),
    /// which is captured for the background sampler thread.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct ToplevelIcon {
        /// Icon name for the XDG icon stock, verbatim (pre-resolution).
        pub name: Option<String>,
        /// The best-sized buffer the client attached, decoded.
        pub pixels: Option<IconPixels>,
    }

    impl ToplevelIcon {
        /// True when the client set neither half — i.e. there is no icon here.
        pub fn is_empty(&self) -> bool {
            self.name.is_none() && self.pixels.is_none()
        }
    }

    /// Sandbox identity derived from cgroup / namespace inspection.
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum SandboxIdentity {
        None,
        Flatpak { app_id: String },
        Snap { instance_name: String },
        OtherContainer { hint: String },
    }
}

/// `/proc/<pid>/cgroup` parsing into a sandbox identity.
pub mod sandbox {
    use crate::values::SandboxIdentity;

    /// Recognized patterns:
    /// - `app-flatpak-<id>-<pid>.scope` → Flatpak
    /// - `snap.<name>.<command>.<uuid>.scope` → Snap
    /// - anything else → None
    pub fn parse_sandbox(cgroup_line: &str) -> SandboxIdentity {
        for segment in cgroup_line.split('/') {
            if let Some(rest) = segment.strip_prefix("app-flatpak-") {
                if let Some(id_end) = rest.rfind('-') {
                    let id = &rest[..id_end];
                    return SandboxIdentity::Flatpak {
                        app_id: id.to_string(),
                    };
                }
            }
            if let Some(rest) = segment.strip_prefix("snap.") {
                if let Some(name_end) = rest.find('.') {
                    let name = &rest[..name_end];
                    return SandboxIdentity::Snap {
                        instance_name: name.to_string(),
                    };
                }
            }
        }
        SandboxIdentity::None
    }
}
