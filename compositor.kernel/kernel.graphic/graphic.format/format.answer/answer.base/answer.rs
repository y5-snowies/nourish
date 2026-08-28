//! THE function. One exhaustive match from "who is asking" to "which formats".
//!
//! Every consumer of a format in this compositor appears in [`Consumer`], and
//! [`available`] has an arm for each. There is no wildcard arm, deliberately and for
//! the same reason the Vulkan fourcc table has none: adding a consumer is a
//! compile error until someone decides what it may use. A default arm is exactly
//! how a new producer silently inherits `Argb8888` and nobody notices for a year.
//!
//! # Consumers name a USE, never a format
//!
//! This is the whole encapsulation. A caller says `available(Consumer::IcedInline)`;
//! it never says `Argb8888`, never queries a device, and never intersects
//! anything. The policy for all of them is readable here in one sitting, which is
//! the property that was missing when the same decisions were spread over twenty
//! call sites in eight crates.

use compositor_kernel_graphic_format_registrar_base::registrar::Registrar;
use compositor_kernel_graphic_format_resolve_base::resolve;
use compositor_kernel_graphic_format_role_base::role::Role;
use compositor_kernel_graphic_format_rule_base::rule::Outcome;
use compositor_kernel_graphic_format_catalog_base::catalog;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Fourcc, Modifier};

/// Everyone who needs to know a format. Exhaustive; no wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consumer {
    /// The `zwp_linux_dmabuf_v1` set advertised to clients.
    ClientFeedback,
    /// The scanout swapchain's candidate set, before smithay walks the ladder.
    ScanoutSwapchain,
    /// The primary plane's hardware cursor buffer.
    CursorPlane,
    /// The CPU-side XCursor image handed to `MemoryRenderBuffer`.
    CursorImage,
    /// iced UI drawn on the compositor thread (builds a `GlesTexture` per slot).
    IcedInline,
    /// iced UI drawn off-thread (no GLES view — the compositor imports natively).
    IcedWorker,
    /// bevy scene drawn on the compositor thread.
    BevyInline,
    /// bevy scene drawn off-thread.
    BevyWorker,
    /// The parallax/shader background worker, which owns both ends of its buffer.
    BackgroundWorker,
    /// A screen-capture entry buffer.
    CaptureEntry,
    /// A screen-capture snapshot copy.
    CaptureSnapshot,
    /// The overview's blur chain.
    OverviewBlur,
    /// The Vulkan renderer's own offscreen output target (nested presentation).
    VulkanOutputTarget,
    /// The `wl_shm` formats advertised to clients.
    ShmAdvertise,
    /// The `wl_shm` formats the renderer can actually upload.
    ShmUpload,
}

/// What a consumer gets. Three shapes, because the questions genuinely differ —
/// and `wl_shm` is a separate variant so the two universes cannot be mixed by
/// accident: shm buffers have no modifiers at all, by protocol.
#[derive(Debug, Clone)]
pub enum Answer {
    /// A whole `(fourcc, modifier)` set — advertisement and the swapchain.
    Set { set: FormatSet, outcome: Outcome },
    /// One fourcc and its ranked modifier candidates — every allocating producer.
    /// An empty `modifiers` with a `Refused` outcome means: do not allocate.
    Choice { fourcc: Fourcc, modifiers: Vec<Modifier>, outcome: Outcome },
    /// `wl_shm` formats. No modifiers, by protocol.
    Shm(&'static [Fourcc]),
    /// ONE device both writes and samples this buffer, so NO cross-API
    /// intersection applies and the full modifier list that device reports is
    /// legal — a wider set than any producer whose buffer crosses an API boundary
    /// may use.
    ///
    /// Distinct from a `Choice` with an empty modifier list, which means REFUSED.
    /// Conflating the two would turn the widest case into the most restrictive
    /// one and stop the compositor at the allocator.
    DeviceLocal { fourcc: Fourcc },
}

impl Answer {
    /// The `(fourcc, modifiers)` of a [`Answer::Choice`], or `None` for the other
    /// shapes. Convenience for the allocating producers, which are all `Choice`.
    pub fn choice(&self) -> Option<(Fourcc, &[Modifier])> {
        match self {
            Self::Choice { fourcc, modifiers, .. } => Some((*fourcc, modifiers)),
            _ => None,
        }
    }

    /// The fourcc of a [`Answer::DeviceLocal`] — one device writes and samples it,
    /// so the caller pairs this with that device's own full modifier list.
    pub fn device_local(&self) -> Option<Fourcc> {
        match self {
            Self::DeviceLocal { fourcc } => Some(*fourcc),
            _ => None,
        }
    }
}

/// The `wl_shm` formats. ONE list, where there used to be three that disagreed:
/// the global advertised `Bgr888`, which neither `mem_formats()` nor the upload
/// cache could handle, so a client taking that offer got nothing.
const SHM: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888, Fourcc::Abgr8888, Fourcc::Xbgr8888];

/// WHICH FORMATS MAY THIS CONSUMER USE? The whole policy, in one match.
///
/// The producers currently all resolve to `Argb8888` because the wgpu texture
/// format they render through is fixed at `Bgra8UnormSrgb`, which only that fourcc
/// reinterprets correctly. That pin is stated ONCE here instead of eight times
/// across the tree, which is what makes unpinning it a change to this function
/// rather than an archaeology exercise.
pub fn available(r: &Registrar, consumer: Consumer) -> Answer {
    // ONE snapshot for the whole answer: every term below comes from the same
    // moment, so a device registering mid-answer cannot half-apply.
    let v = r.view();
    guard(r, &v, consumer);
    let a = arm(r, consumer, &v);
    note(consumer, &a);
    a
}

/// REFUSE TO ANSWER ACROSS AN INCOMPLETE REGISTRAR.
///
/// The check is on the REGISTRAR, not on this consumer's terms. That distinction
/// is the whole point: an answer is not entitled to know which roles it happens
/// to read, and making it declare them puts the invariant in the worst possible
/// place — every arm would have to be kept in step with a second list, and the
/// first arm that gained a term without gaining a list entry would silently
/// reopen the race. There is one question here, asked once: is the machine fully
/// described yet?
///
/// # Why "not yet" is not the same as "never"
///
/// [`View`] sees only "no entry", and `resolve` DEGRADES on it — drops the term
/// and answers from what is left. That is RIGHT for a role nobody will ever
/// publish (nested has no scanout device, and intersecting against an empty set
/// would refuse everything) and WRONG for one that is still coming: the answer
/// then omits a constraint the machine really imposes, comes out wider than the
/// truth, and the producer allocates a modifier the missing party cannot import.
/// Nothing fails at the allocation. It fails later and elsewhere, as a surface
/// that never appears.
///
/// `registrar::REQUIRED` is what separates the two, and it is a constant in that
/// file rather than something a caller declares: "when do I arm the gate" would
/// itself be an ordering decision, and an ordering decision made by a caller is
/// the class of bug this exists to catch. There is no window here — the
/// requirement holds from the first instruction of the process, and the startup
/// sequence is what has to satisfy it.
fn guard(r: &Registrar, v: &compositor_kernel_graphic_format_registrar_base::registrar::View, consumer: Consumer) {
    let missing = r.missing(v);
    if missing.is_empty() {
        return;
    }
    let have: Vec<&'static str> = v.entries().iter().map(|e| e.role.label()).collect();
    fatal!(
        "format answer: {consumer:?} was asked while the registrar is INCOMPLETE — {} \
         required role(s) have not registered: {}. Every backend publishes these \
         unconditionally (`Role::ALL`; a machine with none of a role registers it EMPTY), so their absence is a startup ORDERING \
         defect, not a device limitation: answering now would drop those terms and hand out \
         a wider set than the machine allows, which fails at import as a surface that never \
         appears rather than here. Registered so far: [{}]. Move the registration above this \
         call, or the call below the registration.",
        missing.len(),
        missing.join(", "),
        if have.is_empty() { "nothing".to_string() } else { have.join(", ") },
    );
}

fn arm(r: &Registrar, consumer: Consumer, v: &compositor_kernel_graphic_format_registrar_base::registrar::View) -> Answer {
    match consumer {
        Consumer::ClientFeedback => {
            let set = resolve::advertise(r, v, v.set_or_empty(Role::GlesSample));
            let outcome = if set.indexset().is_empty() {
                Outcome::Refused("the colour policy withholds every importable pair")
            } else {
                Outcome::Ok
            };
            Answer::Set { set, outcome }
        }

        Consumer::ScanoutSwapchain => {
            let (set, outcome) = resolve::scanout(
                v,
                v.set_or_empty(Role::ScanoutEgl),
                r.split_device(),
            );
            Answer::Set { set, outcome }
        }

        // The one universal DRM format. Every KMS driver accepts a linear ARGB8888
        // cursor and the plane is too small for tiling to matter, so this is an
        // assertion rather than a negotiation — but it is stated HERE, so the rule
        // that no crate names a format outside this layer stays absolute.
        Consumer::CursorPlane | Consumer::CursorImage => {
            let (fourcc, modifiers) = constant(consumer);
            Answer::Choice { fourcc, modifiers, outcome: Outcome::Ok }
        }

        // Inline producers build a GlesTexture per slot, so GLES importability is
        // a hard requirement for them and not for the workers.
        Consumer::IcedInline => producer(r, v, INLINE, Role::WgpuImport, "iced-inline"),
        Consumer::BevyInline => producer(r, v, INLINE, Role::WgpuImport, "bevy-inline"),
        Consumer::CaptureEntry => producer(r, v, INLINE, Role::WgpuImport, "capture-entry"),
        Consumer::CaptureSnapshot => producer(r, v, INLINE, Role::WgpuImport, "capture-snapshot"),

        Consumer::IcedWorker => producer(r, v, WORKER, Role::WgpuImport, "iced-worker"),
        Consumer::BevyWorker => producer(r, v, WORKER, Role::WgpuImport, "bevy-worker"),

        // GLES writes the blur chain, so GLES is the second term and there is no
        // wgpu involved at all — pairing it against the wgpu set would be a
        // constraint nothing on this path is subject to.
        Consumer::OverviewBlur => producer(r, v, WORKER, Role::GlesSample, "overview-blur"),

        // BOTH ends on one VkDevice — rendered and sampled by our own Vulkan — so
        // no cross-API intersection applies and the device's full modifier list is
        // legal. The fourcc still follows the session so a 10-bit scanout is not
        // fed an 8-bit source. See `Answer::DeviceLocal`.
        Consumer::BackgroundWorker => {
            let fourcc = r.scanout_fourcc()
                .filter(|c| catalog::is_deep(*c) && catalog::allocatable(*c))
                .unwrap_or(Fourcc::Argb8888);
            Answer::DeviceLocal { fourcc }
        }

        Consumer::VulkanOutputTarget => Answer::DeviceLocal { fourcc: Fourcc::Argb8888 },

        Consumer::ShmAdvertise | Consumer::ShmUpload => Answer::Shm(SHM),
    }
}

/// Log what a set-shaped or device-local consumer got. The producer arms log
/// inside `resolve`, where the per-term counts are.
fn note(consumer: Consumer, a: &Answer) {
    match a {
        Answer::Set { set, outcome } => {
            let mut c: Vec<u32> = set.iter().map(|f| f.code as u32).collect();
            c.sort_unstable();
            c.dedup();
            info!(
                "format available: {consumer:?} -> {} pair(s) over {} fourcc(s) [{outcome:?}]",
                set.iter().count(),
                c.len()
            );
            compositor_kernel_graphic_format_audit_base::audit::dump("    ", set);
        }
        Answer::DeviceLocal { fourcc } => info!(
            "format available: {consumer:?} -> {fourcc:?}, DEVICE-LOCAL (one device writes \
             and samples it, so its full modifier list is legal — no cross-API narrowing)"
        ),
        Answer::Shm(list) => info!(
            "format available: {consumer:?} -> {} shm format(s): {}",
            list.len(),
            list.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(" ")
        ),
        Answer::Choice { .. } => {}
    }
}

/// Terms on the renderer side. Inline producers build a `GlesTexture` per slot, so
/// GLES importability is a hard requirement for them and not for the workers.
const INLINE: &[Role] = &[Role::Sample, Role::GlesSample];
const WORKER: &[Role] = &[Role::Sample];

/// A producer's answer: the fourcc it is pinned to, plus the modifiers every
/// participant agrees on.
fn producer(r: &Registrar, v: &compositor_kernel_graphic_format_registrar_base::registrar::View, terms: &[Role], other: Role, what: &'static str) -> Answer {
    let fourcc = Fourcc::Argb8888;
    let modifiers = resolve::producer_modifiers(r, v, fourcc, terms, other, what);
    let outcome = if modifiers.is_empty() {
        Outcome::Refused("no modifier is shared by every participant for this fourcc")
    } else {
        Outcome::Ok
    };
    Answer::Choice { fourcc, modifiers, outcome }
}

/// The `wl_shm` formats, as fourccs. ONE list for the advertisement, the upload
/// path and the shm cache — where there used to be three that disagreed: the
/// global offered `Bgr888`, which neither `mem_formats()` nor the upload cache
/// could handle, so a client that took that offer got nothing.
pub fn shm() -> &'static [Fourcc] {
    // The shm list needs no device answer, so it needs no handle.
    match Answer::Shm(SHM) {
        Answer::Shm(list) => list,
        _ => SHM,
    }
}

/// The `wl_shm` formats to advertise BEYOND smithay's defaults.
///
/// `ShmState::new` always offers `Argb8888` and `Xrgb8888` — the protocol
/// requires them — and takes only the extras, so handing it the whole list would
/// double-advertise the two.
pub fn shm_extra() -> Vec<smithay::reexports::wayland_server::protocol::wl_shm::Format> {
    shm()
        .iter()
        .filter(|c| !matches!(c, Fourcc::Argb8888 | Fourcc::Xrgb8888))
        .filter_map(|c| smithay::wayland::shm::fourcc_to_shm_format(*c))
        .collect()
}

/// The answer for a consumer whose formats are a CONSTANT — no device is
/// consulted, so no [`Registrar`] is needed.
///
/// The cursor plane and the XCursor image are assertions, not negotiations: every
/// KMS driver takes a linear ARGB cursor, and XCursor pixels are ARGB by
/// specification. They still come from this layer, so the rule that no crate
/// outside it names a format holds — but requiring a device handle for an answer
/// no device contributed to would be ceremony, and ceremony is what gets skipped.
///
/// Anything that DOES depend on a device must use [`available`] and be handed a
/// handle; the match below is the same one either way.
pub fn constant(consumer: Consumer) -> (Fourcc, Vec<Modifier>) {
    match consumer {
        Consumer::CursorPlane => (Fourcc::Argb8888, vec![Modifier::Linear]),
        Consumer::CursorImage => (Fourcc::Argb8888, Vec::new()),
        // Device-local: the FOURCC is a constant; the modifier list comes from
        // the device the caller already holds, so none is returned here.
        Consumer::VulkanOutputTarget => (Fourcc::Argb8888, Vec::new()),
        // Not a constant: it needs a device answer. Empty is a refusal, which the
        // allocator turns into a clear failure rather than a wrong buffer.
        _ => (Fourcc::Argb8888, Vec::new()),
    }
}

/// The `(fourcc, modifiers)` for an ALLOCATING producer.
///
/// A thin typed accessor over [`available`], not a second policy site: every
/// producer arm of that match returns [`Answer::Choice`], so this is total for the
/// consumers it is meant for and degrades to "no modifiers" — which the allocator
/// already treats as a refusal — for any that is not.
pub fn producer_formats(r: &Registrar, consumer: Consumer) -> (Fourcc, Vec<Modifier>) {
    match available(r, consumer) {
        Answer::Choice { fourcc, modifiers, .. } => (fourcc, modifiers),
        // A device-local buffer has no negotiated list by design; its caller uses
        // `Answer::device_local` and its own device's modifiers instead.
        Answer::DeviceLocal { fourcc } => (fourcc, Vec::new()),
        _ => (Fourcc::Argb8888, Vec::new()),
    }
}
