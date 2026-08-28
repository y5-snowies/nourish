//! WHICH DEVICE can do WHAT with WHICH `(fourcc, modifier)` pairs.
//!
//! Kernel bootstrap data, filled in during startup as each device is built.
//! Nothing here relates to a y5 `world` — no `MAIN_WORLD`, no per-world state; a
//! GPU's format capability is a property of the machine and outlives every world.
//!
//! One table, one shape:
//!
//! ```text
//! registrar.register(Device::of(&node), Role::Scanout, pairs, "scanout egl")
//! ```
//!
//! # Owned, not global
//!
//! The state lives in a `StorageConcurrent` slot addressed by the same `Token`
//! type the rest of the kernel uses, and a [`Registrar`] handle is given to each
//! construction site that needs one. No process-wide singleton, and no bare
//! `static` per subsystem — which is how a tree ends up with several unrelated
//! globals nobody can enumerate.
//!
//! It cannot be the per-world `Storage`: that holds `Box<dyn Any>` behind
//! `&mut self` and is reached through `&Loop` on the compositor thread, while this
//! is read from the iced, bevy and background WORKER threads every time one
//! allocates a buffer. `StorageConcurrent` is the thread-safe sibling built for
//! exactly that.
//!
//! # Atomicity
//!
//! Every answer is computed from a single [`View`] — one snapshot, one lock
//! acquisition. That is the guarantee: the terms of a producer's intersection
//! cannot come from different moments, so a device registering mid-answer can
//! neither half-apply nor tear.
//!
//! # Reading before registering
//!
//! A consumer of a missing entry DEGRADES — it drops the term rather than
//! intersecting against an empty set, because intersecting would refuse
//! everything. That is correct and must stay; what matters is that it SAYS so,
//! which `format.audit` does. The cost was measured: on winit the advertisement
//! runs before the renderer registers, so the compositor advertised 644 pairs to
//! clients while being able to import 154.

use compositor_kernel_graphic_format_role_base::role::Role;
use compositor_support_system_storage_concurrent_base::concurrent::StorageConcurrent;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmNode, NodeType};
use std::sync::Arc;

/// A GPU, by its DRM `dev_t`.
///
/// `dev_t` rather than a path because that is what the protocol uses for
/// `main_device`, what smithay keys its GPU manager by, and the one identifier
/// that survives a `/dev/dri/card1` vs `renderD128` mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Device(pub u64);

impl Device {
    /// A device with no resolved DRM node — a wgpu adapter probed before any node
    /// is known, or the nested backend, which has none. Still a real
    /// registration; it simply cannot be told apart from another such device.
    pub const UNSPECIFIED: Device = Device(0);

    pub fn of(node: &DrmNode) -> Self {
        Device(node.dev_id())
    }
}

/// One device's answer for one role.
#[derive(Debug, Clone)]
pub struct Entry {
    pub device: Device,
    pub role: Role,
    pub set: FormatSet,
    pub source: &'static str,
}

/// ALL of this layer's kernel state, in one place.
///
/// One object rather than a table plus a handful of loose atomics: every field is
/// bootstrap data written once during startup and read together, and separate
/// globals are separate things to keep consistent with no type saying they belong
/// to each other.
#[derive(Default)]
pub struct Kernel {
    entries: Vec<Entry>,
    /// Whether the colour-managed composite is running — gates the
    /// extended-range formats. See `format.catalog`'s `expressible`.
    color_managed: bool,
    /// Composite and scanout are different GPUs. Only affects how a refusal is
    /// EXPLAINED — a cross-vendor split is the common cause and deserves naming —
    /// never what is chosen.
    split_device: bool,
    /// The fourcc each CRTC's swapchain ACTUALLY got. Per-CRTC, not one value:
    /// see [`Registrar::scanout_fourcc`].
    scanout: Vec<(u64, Fourcc)>,
}

compositor_support_system_storage_token_base::y5_storage!(pub KERNEL, KERNEL_MUT: Kernel);

/// Where the compositor thread finds the [`Registrar`] handle.
///
/// Created ONCE by the loader and inserted into the kernel `Storage`; everything
/// holding a `&Loop` reads it from here, and everything that does not (the wgpu
/// contexts, the Vulkan renderer, the background worker) is handed a clone at
/// construction. The token lives WITH the type it addresses, like `KERNEL` above:
/// it used to sit in `orchestration.core.state` beside `GPU_BINDING`, which meant
/// the orchestration state crate had to name a kernel format type to declare a
/// token it never reads.
pub static FORMATS: compositor_support_system_storage_token_base::base::Token<Registrar> =
    compositor_support_system_storage_token_base::base::Token::new();

/// WHAT A COMPLETE REGISTRAR LOOKS LIKE: every role there is. `format.answer`
/// will not answer anything until all of them have registered.
///
/// # Why the whole enum, with no exceptions
///
/// Because an exception is a hole with a plausible reason attached, and the holes
/// are precisely where this went wrong before. "Nested has no scanout device" is
/// true, but it is a statement the machine should MAKE, not one the gate should
/// assume — a backend that has nothing for a role registers an EMPTY set and says
/// so, which reads identically to every consumer (`set_or_empty` → empty →
/// degrade, exactly as before) while being distinguishable from silence to the
/// gate. Exempting the role instead would mean a backend that genuinely SHOULD
/// have published it, and did not, is answered for anyway.
///
/// [`Role::ALL`] is a fixed-length array over the enum, so adding a role is a
/// compile error until someone decides who registers it.
///
/// # Why this is a constant here and not something a backend declares
///
/// Because "when do I arm the gate" is itself an ordering decision, and an
/// ordering decision made by a caller is exactly the class of bug this exists to
/// catch. A backend that armed late would leave the window it was meant to close;
/// one that armed early would fail its own bring-up. Stated here, there is no
/// window and no announcement to get wrong — the requirement is true from the
/// first instruction of the process, and the startup sequence is what has to
/// satisfy it.
///
/// # Why "not yet" is not "never"
///
/// [`View`] sees only "no entry", and `format.resolve` DEGRADES on it — drops the
/// term and answers from what is left. That is RIGHT for a role nobody will ever
/// publish and WRONG for one that is still coming: the answer omits a constraint
/// the machine really imposes, comes out wider than the truth, and the producer
/// allocates a modifier the missing party cannot import. Nothing fails at the
/// allocation; it fails later and elsewhere, as a surface that never appears.
/// Requiring an explicit empty registration is what separates the two cases.
const REQUIRED: &[Role] = &[Role::Sample, Role::GlesSample, Role::WgpuImport];

/// A handle to the kernel's format data. Cheap to clone (one `Arc`); every method
/// takes `&self`, so a producer holds one for its lifetime.
#[derive(Clone, Debug, Default)]
pub struct Registrar {
    store: Arc<StorageConcurrent>,
}

impl Registrar {
    /// A fresh, empty registrar. Created ONCE at boot; the handle is what travels.
    pub fn new() -> Self {
        Self { store: Arc::new(StorageConcurrent::new()) }
    }

    fn read<T: Default>(&self, f: impl FnOnce(&Kernel) -> T) -> T {
        self.store.with(&KERNEL, f).unwrap_or_default()
    }

    /// `entry`: the slot fills itself on first use, so no caller has to sequence
    /// "create the slot" before "register a device".
    fn write<T>(&self, f: impl FnOnce(&mut Kernel) -> T) -> T {
        self.store.entry(&KERNEL_MUT, f)
    }

    /// Record what `device` can do in `role`. Called once per (device, role),
    /// where that device is built.
    ///
    /// Re-registering the SAME pairs is not a fault — two producers can
    /// legitimately probe one adapter and get one result. A genuine disagreement
    /// is, because it means two devices are claiming one capability.
    pub fn register(&self, device: Device, role: Role, set: FormatSet, source: &'static str) {
        let pairs = set.iter().count();
        let mut codes: Vec<u32> = set.iter().map(|f| f.code as u32).collect();
        codes.sort_unstable();
        codes.dedup();
        let fourccs = codes.len();
        self.write(|k| {
            if let Some(prev) = k.entries.iter_mut().find(|e| e.device == device && e.role == role) {
                if prev.set.indexset() != set.indexset() {
                    warn!(
                        "format registrar: {} on device {:#x} re-registered with a DIFFERENT \
                         set ({pairs} pair(s) from {source}, was {} from {}) — one device, one \
                         answer per role; the later value wins but the two disagree by definition",
                        role.label(), device.0, prev.set.iter().count(), prev.source
                    );
                    prev.set = set;
                    prev.source = source;
                }
                return;
            }
            info!(
                "format registrar: REGISTER device={:#x} role={} — {pairs} pair(s) over \
                 {fourccs} fourcc(s) [{source}]",
                device.0, role.label()
            );
            compositor_kernel_graphic_format_audit_base::audit::dump("    ", &set);
            k.entries.push(Entry { device, role, set, source });
        })
    }

    /// Declared roles with no entry in `v`. Empty means the registrar is
    /// COMPLETE, which is the only state in which `format.answer` will answer.
    pub fn missing(&self, v: &View) -> Vec<&'static str> {
        Role::ALL
            .iter()
            .filter(|role| v.set(**role).is_none())
            .map(|role| role.label())
            .collect()
    }

    /// Register that this machine has NOTHING for `role` — an ANSWER, not a gap.
    ///
    /// Nested has no scanout device; no plane exists off the native path. Those
    /// are facts, and stating them costs one call and makes the difference
    /// between "there is none" and "it has not arrived yet" visible to
    /// [`Self::missing`]. Every consumer already reads an empty set the same way
    /// it reads a missing one, so this changes no answer.
    pub fn absent(&self, role: Role, why: &'static str) {
        self.register(Device::UNSPECIFIED, role, FormatSet::default(), why);
    }

    /// Snapshot the table. One lock acquisition, one consistent set of answers.
    ///
    /// An empty snapshot when the slot has not been filled yet is the correct
    /// answer, not an error: nothing has registered, so every consumer degrades.
    pub fn view(&self) -> View {
        View(self.read(|k| k.entries.clone()))
    }

    /// The table, as a block for the log: what every device claims it can do, so
    /// "what is available" is one line per (device, role) rather than an
    /// inference across eight crates.
    pub fn overview(&self) {
        let v = self.view();
        info!("format registrar: {} (device, role) registration(s)", v.entries().len());
        for e in v.entries() {
            let mut codes: Vec<u32> = e.set.iter().map(|f| f.code as u32).collect();
            codes.sort_unstable();
            codes.dedup();
            info!(
                "  device={:#x} {:<17} {:>4} pair(s) over {:>3} fourcc(s)  [{}]",
                e.device.0, e.role.label(), e.set.iter().count(), codes.len(), e.source
            );
        }
    }

    pub fn set_color_managed(&self, now: bool) {
        self.write(|k| k.color_managed = now);
    }

    pub fn color_managed(&self) -> bool {
        self.read(|k| k.color_managed)
    }

    pub fn set_split_device(&self, now: bool) {
        self.write(|k| k.split_device = now);
    }

    pub fn split_device(&self) -> bool {
        self.read(|k| k.split_device)
    }

    /// Record the fourcc CRTC `crtc`'s swapchain ACTUALLY got, once smithay has
    /// chosen from the offered ladder.
    ///
    /// The achieved format, not the requested depth, is the honest answer to "is
    /// this a 10-bit session": it is only 10-bit if deep colour was asked for AND
    /// the plane accepted it AND a mode was found.
    pub fn set_scanout_fourcc(&self, crtc: u64, fourcc: Fourcc) {
        self.write(|k| match k.scanout.iter_mut().find(|(c, _)| *c == crtc) {
            Some(slot) => slot.1 = fourcc,
            None => k.scanout.push((crtc, fourcc)),
        })
    }

    /// Forget a CRTC's format. Called when its pipe goes away, so an unplugged
    /// monitor stops influencing [`Self::scanout_fourcc`].
    pub fn forget_scanout(&self, crtc: u64) {
        self.write(|k| k.scanout.retain(|(c, _)| *c != crtc));
    }

    /// The format a producer should match, across every live CRTC.
    ///
    /// The DEEPEST wins, not the last one built. Pipes are per-monitor while this
    /// answer is one value, and taking the most recent made a session's colour
    /// depth depend on plug order: attach an 8-bit panel beside a 10-bit one and
    /// every producer silently dropped to 8-bit, banding the 10-bit display for
    /// the sake of a monitor that could not have shown the difference.
    ///
    /// Preferring the deepest costs the 8-bit panel nothing — the scanout
    /// swapchain still runs at whatever its own plane accepted, and this only
    /// decides what PRODUCERS render into. Among equal depths the first
    /// registered wins, which is stable across replug.
    pub fn scanout_fourcc(&self) -> Option<Fourcc> {
        self.read(|k| {
            k.scanout
                .iter()
                .map(|(_, f)| *f)
                .max_by_key(|f| compositor_kernel_graphic_format_catalog_base::catalog::depth(*f))
        })
    }
}

/// A consistent snapshot of the whole table.
///
/// Every answer is computed from one of these. Taking the terms of an
/// intersection from separate reads is the race this type exists to remove.
pub struct View(Vec<Entry>);

impl View {
    /// The pairs for `role` across every device that registered it.
    ///
    /// A union rather than a pick, because a role is asked of the machine and not
    /// of a card: there is one composite and one scanout, and where two adapters
    /// register `WgpuImport` they are the same physical device probed twice.
    /// `None` means nothing registered it, which callers must DEGRADE on.
    pub fn set(&self, role: Role) -> Option<FormatSet> {
        let mut it = self.0.iter().filter(|e| e.role == role).peekable();
        it.peek()?;
        let mut acc: Vec<smithay::backend::allocator::Format> = Vec::new();
        for e in it {
            acc.extend(e.set.iter().copied());
        }
        Some(acc.into_iter().collect())
    }

    /// The pairs for exactly one device and role.
    pub fn set_for(&self, device: Device, role: Role) -> Option<FormatSet> {
        self.0
            .iter()
            .find(|e| e.device == device && e.role == role)
            .map(|e| e.set.clone())
    }

    /// [`Self::set`], or an empty set. An empty result means the answer built on
    /// it is wider than the truth; `format.audit` is what says so.
    pub fn set_or_empty(&self, role: Role) -> FormatSet {
        self.set(role).unwrap_or_default()
    }

    /// Entries, for the overview.
    pub fn entries(&self) -> &[Entry] {
        &self.0
    }
}

/// Which GPU the compositor composites on: `render_node`, else `fallback` (the
/// scanout device) when it names nothing usable.
///
/// ONE definition, because several places have to agree and there is no error if
/// they do not. The compositor ADVERTISES a device to clients, VALIDATES the
/// buffers they send, and DRAWS them. Let those disagree and a client allocates on
/// one GPU, passes validation on a second and fails to import on a third — which
/// reaches the user as blank windows, not as anything anyone can act on.
pub fn composite_node(fallback: DrmNode) -> DrmNode {
    let path = compositor_model_environment_config_base::base::get().render_node.clone();
    DrmNode::from_path(&path)
        .ok()
        .map(|n| n.node_with_type(NodeType::Render).and_then(|r| r.ok()).unwrap_or(n))
        .unwrap_or(fallback)
}
