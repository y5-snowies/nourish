//! What a capability set is an answer TO.
//!
//! A bare `FormatSet` does not say which question it answers, and the tree had
//! six different answers in flight at once — what the composite can SAMPLE, what
//! it can COLOUR-ATTACH, what the scanout device's EGL can render into, what the
//! wgpu adapter can import, what GLES can sample, and what the primary plane will
//! actually scan out. Two of those were published into separate globals from two
//! separate points in one function and fed two separate intersections that were
//! never reconciled.
//!
//! Tagging the set with its [`Role`] makes "which set is this" a property of the
//! value rather than of the variable name it happened to be assigned to, and
//! makes a wrong-set intersection something a reader can see.
//!
//! The tagging itself lives in `format.registrar`'s `Entry`, which carries the
//! role, the set, the device and the provenance together. A `Caps` type here once
//! did the same job for a value in flight; nothing ever constructed one, so it is
//! gone rather than left as a second shape for the same idea.

/// The question a [`Caps`] answers. One variant per genuinely distinct capability
/// in the tree — not per device, because two roles can live on one device and
/// routinely disagree (`Sample` and `Render` are both the composite's Vulkan
/// device and are enumerated with different feature flags over different fourcc
/// universes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The composite can SAMPLE a client dmabuf through this pair.
    /// Vulkan `SAMPLED_IMAGE` over the whole importable table.
    Sample,
    /// The composite can COLOUR-ATTACH this pair. Vulkan `COLOR_ATTACHMENT`,
    /// restricted to the scanout ladder.
    Render,
    /// The SCANOUT device's EGL can render into this pair. The swapchain's
    /// candidate set before narrowing.
    ScanoutEgl,
    /// The wgpu adapter a producer draws with can import this pair as a colour
    /// target.
    WgpuImport,
    /// The GLES renderer can sample this pair. A hard requirement only for the
    /// inline paths, which build a `GlesTexture` per slot.
    GlesSample,
    /// The primary plane will actually scan this pair out (`IN_FORMATS`).
    ///
    /// Enumerated today and thrown away — a modifier can survive every other term
    /// and still be un-scanoutable, which is precisely the class of bug that keeps
    /// recurring here.
    Plane,
}

impl Role {
    /// EVERY role, and the array length makes that a compile-time fact: add a
    /// variant above without adding it here and this stops building.
    ///
    /// `format.registrar` requires all of them to be registered before it will
    /// answer anything, so this list is the definition of a complete registrar.
    /// A machine with nothing for a role registers an EMPTY set for it — "asked,
    /// and there is none" — which is a different statement from silence and is
    /// the one the answer layer can safely degrade on.
    pub const ALL: [Role; 6] = [
        Self::Sample,
        Self::Render,
        Self::ScanoutEgl,
        Self::WgpuImport,
        Self::GlesSample,
        Self::Plane,
    ];

    /// Short label for logs and the audit.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sample => "composite-sample",
            Self::Render => "composite-render",
            Self::ScanoutEgl => "scanout-egl",
            Self::WgpuImport => "wgpu-import",
            Self::GlesSample => "gles-sample",
            Self::Plane => "plane-scanout",
        }
    }
}
