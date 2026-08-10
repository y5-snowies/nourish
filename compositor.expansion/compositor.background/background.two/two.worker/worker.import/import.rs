//! The worker's client-buffer import cache: a client `Dmabuf` in, a `VkImageView`
//! on the WORKER's device out, memoised by buffer identity.
//!
//! Reuses `vulkan.memory/memory.import` — the same importer the compositor runs —
//! so a window means the same thing on both devices rather than two decodings of
//! the same buffer.
//!
//! # Why this needs no fence and no ack
//!
//! The importer DUPS the fd, so an image here stays valid after the client
//! releases the buffer and after the compositor drops its own import. Nothing has
//! to be kept alive on the other side and no lifetime is shared, which is what
//! lets the window set travel as a plain slot rather than a ring with an ack
//! protocol.
//!
//! What it does not get is an acquire fence: a client may be redrawing while the
//! worker samples, so a torn frame is possible. On a background effect that is an
//! artefact, not a fault — and the alternative is making the compositor mediate
//! per-client acquire points across a device boundary, which is the expensive and
//! dangerous design this avoids.
//!
//! Keyed on pointer identity — `Dmabuf::weak()` for a client buffer, the
//! `Arc<Shared>` for a shared image — exactly as the compositor's cache is. Each
//! entry KEEPS a weak reference to whatever it was keyed on, which is what makes
//! the key sound rather than merely convenient: a `Weak` holds the allocation
//! alive, so the address cannot be recycled under a stale entry.

use ash::vk;
use compositor_background_two_worker_device::device::Device;
use compositor_pipeline_abi_worldset_base::base::Source;
use smithay::backend::allocator::dmabuf::WeakDmabuf;
use std::collections::HashMap;

/// Cache key, pointer identity either way.
///
/// The shared arm was the raw fd, on the reasoning that the compositor exports
/// one per uploaded image. It does — but it CLOSES the old one when a surface is
/// reallocated, and the kernel hands the number straight back out. Resize a SHM
/// window and its new export can land on the fd its previous one just freed,
/// matching an entry `reap` never sweeps: the worker then samples the image from
/// before the resize, and keeps doing so until some later reallocation happens to
/// draw a different number.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Client(WeakDmabuf),
    Shared(usize),
}

struct Imported {
    image: vk::Image,
    memory: vk::DeviceMemory,
    extra: Vec<vk::DeviceMemory>,
    view: vk::ImageView,
    /// Held so the `Arc` this entry was keyed on cannot have its address reused
    /// while the entry lives, and so `reap` can tell when the compositor has
    /// dropped it. `None` for a client buffer, whose `WeakDmabuf` key is both.
    shared: Option<std::sync::Weak<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
}

/// Per-worker-device import cache. One per worker, not per pane: the same client
/// window can appear on several panes and must import once.
#[derive(Default)]
pub struct Imports {
    map: HashMap<Key, Imported>,
    /// Evicted, not yet destroyed. A worker submit that is still executing may be
    /// sampling one of these, and the cache is shared across panes so no single
    /// pane's fence proves otherwise. [`release`](Imports::release) is where they
    /// actually go, once the caller can show every pane is idle.
    retired: Vec<Imported>,
}

/// One imported window buffer, ready to be acquired into this frame.
///
/// The IMAGE travels with the view because an imported image arrives in
/// `UNDEFINED` layout and the shader samples it as `SHADER_READ_ONLY_OPTIMAL`;
/// something has to transition it, and only the caller holds a command buffer.
#[derive(Clone, Copy)]
pub struct Acquired {
    pub image: vk::Image,
    pub view: vk::ImageView,
    /// Whether the pixels came from OUTSIDE Vulkan — a client's own dmabuf, whose
    /// producer is another driver. That is the `VK_QUEUE_FAMILY_FOREIGN_EXT` case.
    ///
    /// An `OPAQUE_FD` share of the compositor's own upload is NOT: both ends are
    /// Vulkan, so it would take `VK_QUEUE_FAMILY_EXTERNAL` and a matching RELEASE
    /// on the compositor's queue. This path has no cross-device semaphore by
    /// design, so it performs no ownership transfer for those at all — only the
    /// layout transition, which is what the shader actually requires.
    pub foreign: bool,
}

impl Imports {
    pub fn new() -> Self {
        Self::default()
    }

    /// The worker-device view for `src`, importing on first use.
    ///
    /// Both client types land here. A dmabuf goes through the same importer the
    /// compositor uses; a SHM surface arrives as an `OPAQUE_FD` share of the image
    /// the compositor uploaded into, valid because both logical devices sit on the
    /// same physical device. Either way the fd is DUPED, so this copy outlives the
    /// compositor's and the client's.
    pub fn view(&mut self, d: &Device, src: &Source) -> Result<Acquired, String> {
        let key = match src {
            Source::Client(b) => Key::Client(b.weak()),
            Source::Shared(s) => Key::Shared(std::sync::Arc::as_ptr(s) as usize),
            Source::Unavailable => return Err("window has no importable source".into()),
        };
        let foreign = matches!(src, Source::Client(_));
        if let Some(i) = self.map.get(&key) {
            return Ok(Acquired { image: i.image, view: i.view, foreign });
        }
        let entry = match src {
            Source::Client(b) => {
                let up =
                    compositor_kernel_vulkan_memory_import_base::import::import(&d.dev, &d.phd, b)
                        .map_err(|e| format!("worker import: {e}"))?;
                Imported {
                    image: up.image,
                    memory: up.memory,
                    extra: up.extra_memory,
                    view: up.view,
                    shared: None,
                }
            }
            Source::Shared(s) => {
                let (image, memory, view) =
                    compositor_kernel_vulkan_memory_external_base::external::import(&d.dev, s)
                        .map_err(|e| format!("worker external import: {e}"))?;
                Imported {
                    image,
                    memory,
                    extra: Vec::new(),
                    view,
                    shared: Some(std::sync::Arc::downgrade(s)),
                }
            }
            Source::Unavailable => unreachable!("checked above"),
        };
        let acquired = Acquired { image: entry.image, view: entry.view, foreign };
        self.map.insert(key, entry);
        Ok(acquired)
    }

    /// Evict imports the compositor has dropped — a client buffer whose surface
    /// died, a shared image whose texture was reallocated. Both arms, because a
    /// resized SHM surface mints a new image on every reallocation and leaving the
    /// old one in the map leaked it for the life of the process.
    ///
    /// Evicts only; see [`release`](Imports::release) for why nothing is destroyed
    /// here. Cheap: one liveness check per entry, and the map holds only live
    /// drawables in the steady state.
    pub fn reap(&mut self) {
        let stale: Vec<Key> = self
            .map
            .iter()
            .filter(|(k, v)| match k {
                Key::Client(w) => w.upgrade().is_none(),
                Key::Shared(_) => v.shared.as_ref().is_some_and(|w| w.upgrade().is_none()),
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            if let Some(i) = self.map.remove(&k) {
                self.retired.push(i);
            }
        }
    }

    /// Destroy everything [`reap`](Imports::reap) evicted.
    ///
    /// The caller must have established that NO worker submit is in flight. That
    /// is not a per-pane question: one cache serves every pane, so a pane's own
    /// fence says nothing about whether another pane's submit is still sampling
    /// the image being freed. `serve` checks all of them together, which is both
    /// cheap (a fence poll each) and a proof rather than an estimate.
    pub fn release(&mut self, d: &Device) {
        for i in self.retired.drain(..) {
            destroy(d, &i);
        }
    }

    /// Import one of the compositor's shared band images — `content` or the
    /// `windows` layer. Keyed like any other shared image, so each is imported
    /// once per compositor-side allocation rather than per frame. These are
    /// reallocated on an output resize, so they were exposed to the same recycled
    /// fd as a client surface.
    pub fn shared_view(
        &mut self,
        d: &Device,
        s: &std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>,
    ) -> Result<vk::ImageView, String> {
        let key = Key::Shared(std::sync::Arc::as_ptr(s) as usize);
        if let Some(i) = self.map.get(&key) {
            return Ok(i.view);
        }
        let (image, memory, view) =
            compositor_kernel_vulkan_memory_external_base::external::import(&d.dev, s)
                .map_err(|e| format!("worker shared-band import: {e}"))?;
        self.map.insert(
            key,
            Imported {
                image,
                memory,
                extra: Vec::new(),
                view,
                shared: Some(std::sync::Arc::downgrade(s)),
            },
        );
        Ok(view)
    }

    pub fn destroy_all(&mut self, d: &Device) {
        for (_, i) in self.map.drain() {
            destroy(d, &i);
        }
        self.release(d);
    }
}

fn destroy(d: &Device, i: &Imported) {
    unsafe {
        d.dev.device.destroy_image_view(i.view, None);
        d.dev.device.destroy_image(i.image, None);
        d.dev.device.free_memory(i.memory, None);
        for m in &i.extra {
            d.dev.device.free_memory(*m, None);
        }
    }
}
