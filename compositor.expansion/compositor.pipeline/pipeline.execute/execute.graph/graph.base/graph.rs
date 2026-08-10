//! Multipass background executor: run a `GraphPipeline`'s intermediate passes
//! into owned offscreen targets (outside the main composite pass), then draw its
//! swapchain-output pass inline in the composite pass where the single background
//! `ShaderPass` draws today. Vulkan-only; driven by the opt-in multipass path.
//! Renderer-agnostic of shader specifics — it runs SPIR-V + push bytes handed to
//! it, building/caching an `EffectPass` per pass.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_pipeline_execute_effect_base::effect::EffectPass;
use compositor_pipeline_abi_worldset_base::base::Own;
use compositor_kernel_vulkan_renderer_error_base::VulkanError;
use compositor_pipeline_abi_seam_base::base::{Requirement, Requires};
use std::collections::HashMap;

/// Intermediate-target pixel format (renderer-side mirror of the seam enum).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphFormat {
    Rgba8,
    Rgba16f,
    Rgba32f,
}

impl GraphFormat {
    fn vk(self) -> vk::Format {
        match self {
            GraphFormat::Rgba8 => vk::Format::R8G8B8A8_UNORM,
            GraphFormat::Rgba16f => vk::Format::R16G16B16A16_SFLOAT,
            GraphFormat::Rgba32f => vk::Format::R32G32B32A32_SFLOAT,
        }
    }
}

/// Where a pass writes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphOutput {
    Target(usize),
    Swapchain,
}

/// Input sentinel: a pass input index of `CONTENT` samples the composited scene
/// (`content`) rather than a declared intermediate target. Used by after-content
/// passes (vignette-above, tone-map, glass backdrop). Bound by the caller-supplied
/// content view. `shader.pipeline` re-exports these rather than restating them —
/// two independent declarations of the same sentinel diverge into a pass sampling
/// the wrong built-in image, with nothing to catch it.
pub const CONTENT: usize = usize::MAX;

/// Input sentinel: a pass input index of `HISTORY` samples the previous frame's
/// final composited image (`history`) rather than a declared target. Readable in
/// either band. Bound from the persistent history view the executor keeps + fills
/// when the pipeline requires `previous_frame` (consumption-gated).
pub const HISTORY: usize = usize::MAX - 1;

/// Input sentinel: the composited WINDOW LAYER — client windows over transparency,
/// background excluded.
pub const WINDOWS: usize = usize::MAX - 2;

/// Input sentinel BASE: an input index of `TEXTURE + i` samples the bundle's i-th
/// declared texture rather than a target.
///
/// A base rather than a single value because there are many, and the alternative
/// — a second parallel list of inputs — would mean every consumer of `inputs`
/// learning that a pass's bindings come from two places in an order neither list
/// states. As one index space, a texture is an input like any other: same
/// `@group(0)` binding rule, same sorted-by-name order, same descriptor write.
///
/// `1 << 48` sits above any target index (targets are counted in ones) and far
/// below the built-in sentinels at the top of the range, so the three families
/// cannot meet. It is checked with a range test rather than an equality, which is
/// why it is the only one that has to be matched after the others.
pub const TEXTURE: usize = 1 << 48;

/// One compiled pass: SPIR-V + packed push + resolved input target indices.
///
/// The modules and entry names are `Arc`s shared with the producing element, not
/// owned copies: this is rebuilt on every draw of every frame, once per pass, but
/// the bytes are only read on a pipeline-cache miss. `push` stays owned — it is
/// small and genuinely differs per frame.
pub struct GraphPass {
    pub id: u64,
    pub spv: std::sync::Arc<[u8]>,
    pub vert_spv: Option<std::sync::Arc<[u8]>>,
    pub vert_entry: std::sync::Arc<str>,
    pub frag_entry: std::sync::Arc<str>,
    pub push: Vec<u8>,
    pub inputs: Vec<usize>,
    pub output: GraphOutput,
    /// What this pass requires bound: `world_set()` binds the geometry UBO at
    /// `@group(1) @binding(0)`, `textures()` also the bindless array at
    /// `@binding(1)`.
    pub requires: Requires,
    /// Run every Nth frame, holding the target in between (1 = every frame).
    pub cadence: u32,
}

/// A bundle texture, ready to upload: decoded RGBA8 and what the samples mean.
///
/// The renderer-side mirror of the seam type, kept as its own struct for the same
/// reason `GraphFormat` is: this crate is the executor and does not depend on the
/// dispatch vocabulary in either direction.
pub struct GraphTexture {
    pub width: u32,
    pub height: u32,
    /// `_SRGB` image format when true, so the hardware linearises on sample.
    pub srgb: bool,
    pub pixels: std::sync::Arc<[u8]>,
}

/// A multipass pipeline: intermediate `(format, scale)` targets + the passes in
/// each band. `passes` are `before-content` (background); `after` run once the
/// scene (background + windows) is composited into `content`, which they sample.
pub struct GraphPipeline {
    pub id: u64,
    /// `(format, scale, persist)`. A persistent target is double-buffered by
    /// [`GraphExec`]: the pass writes one image and samples the other, so it reads
    /// the previous frame. See `shader.manifest::Target::persist`.
    pub targets: Vec<(GraphFormat, f32, bool)>,
    /// Declared storage-buffer sizes in bytes, in `@group(2)` binding order.
    pub storage: Vec<u64>,
    /// Declared textures, in the order the `TEXTURE` sentinel indexes them.
    /// Uploaded once when the pipeline is prepared; read-only thereafter.
    pub textures: Vec<GraphTexture>,
    pub passes: Vec<GraphPass>,
    pub after: Vec<GraphPass>,
    /// The union of every pass's `requires` — every engine cost this pipeline
    /// incurs, and nothing else. `previous_frame`/`window_layer` each gate one
    /// persistent image (no VRAM otherwise); `whole_band()` widens the world set
    /// beyond client windows regardless of `owns`.
    pub requires: Requires,
    /// How much of the world band the pipeline composites itself; the compositor
    /// leaves exactly that much out of its own draw (`windows: pipeline` /
    /// `windows: world`, §8d).
    pub owns: compositor_pipeline_abi_worldset_base::base::Own,
    /// The generated grid pass for a per-frame GPU warp map, when the bundle asked
    /// for one. Carried here rather than executed here: it renders into its own
    /// tiny target, not into this graph's, so `renderer.warpmap` owns it.
    pub warp_map: Option<GraphPass>,
}

/// Take ownership of a seam pipeline. The SPIR-V and entry names are `Arc`s
/// shared with the producing element (a refcount bump, not a copy); only `push`
/// is copied, since it must outlive the dispatch call and is ~112 bytes.
///
/// Lives here rather than in the renderer so BOTH consumers share one
/// implementation: the compositor's `submit_frame`, and the off-thread background
/// worker, which runs the same executor on its own device.
pub fn from_seam(
    p: compositor_pipeline_abi_seam_base::base::ShaderPipeline,
) -> GraphPipeline {
    use compositor_pipeline_abi_seam_base::base::{PipelineOutput, PipelinePass, TargetFormat};
    let fmt = |f: TargetFormat| match f {
        TargetFormat::Rgba8 => GraphFormat::Rgba8,
        TargetFormat::Rgba16f => GraphFormat::Rgba16f,
        TargetFormat::Rgba32f => GraphFormat::Rgba32f,
    };
    let own_pass = |pass: PipelinePass| GraphPass {
        id: pass.variant.id,
        spv: pass.variant.spv,
        vert_spv: pass.variant.vert_spv,
        vert_entry: pass.variant.vert_entry,
        frag_entry: pass.variant.frag_entry,
        push: pass.variant.push.to_vec(),
        inputs: pass.inputs,
        output: match pass.output {
            PipelineOutput::Target(i) => GraphOutput::Target(i),
            PipelineOutput::Swapchain => GraphOutput::Swapchain,
        },
        requires: pass.requires,
        cadence: pass.cadence.max(1),
    };
    GraphPipeline {
        id: p.id,
        targets: p.targets.iter().map(|t| (fmt(t.format), t.scale, t.persist)).collect(),
        storage: p.storage.clone(),
        textures: p
            .textures
            .iter()
            .map(|t| GraphTexture {
                width: t.width,
                height: t.height,
                srgb: t.srgb,
                // Refcount bump. The bytes are read once, on the frame the upload
                // is recorded, but the seam is crossed every frame.
                pixels: t.pixels.clone(),
            })
            .collect(),
        passes: p.passes.into_iter().map(own_pass).collect(),
        after: p.after.into_iter().map(own_pass).collect(),
        requires: p.requires,
        warp_map: p.warp_map.map(|v| GraphPass {
            id: v.id,
            spv: v.spv,
            vert_spv: v.vert_spv,
            vert_entry: v.vert_entry,
            frag_entry: v.frag_entry,
            push: v.push.to_vec(),
            // The grid pass samples nothing and writes its own target, so the
            // fields that describe a graph pass's place in the graph are inert.
            inputs: Vec::new(),
            output: GraphOutput::Swapchain,
            requires: Requires::NONE,
            cadence: 1,
        }),
        owns: p.owns.into(),
    }
}

impl GraphPipeline {
    /// Whether this pipeline has any after-content passes (⇒ the scene must be
    /// composited offscreen into `content` so they can sample it).
    pub fn has_after(&self) -> bool {
        !self.after.is_empty()
    }
}

struct TargetImg {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    w: u32,
    h: u32,
    format: vk::Format,
}

impl TargetImg {
    fn destroy(&self, dev: &VulkanDevice) {
        unsafe {
            dev.device.destroy_image_view(self.view, None);
            dev.device.destroy_image(self.image, None);
            dev.device.free_memory(self.memory, None);
        }
    }
}

fn device_local(mem: &vk::PhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    (0..mem.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && mem.memory_types[i as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    })
}

fn range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn barrier(
    dev: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    ss: vk::PipelineStageFlags2,
    ds: vk::PipelineStageFlags2,
    sa: vk::AccessFlags2,
    da: vk::AccessFlags2,
) {
    let b = vk::ImageMemoryBarrier2::default()
        .src_stage_mask(ss)
        .dst_stage_mask(ds)
        .src_access_mask(sa)
        .dst_access_mask(da)
        .old_layout(old)
        .new_layout(new)
        .image(image)
        .subresource_range(range());
    unsafe {
        dev.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(std::slice::from_ref(&b)),
        );
    }
}

/// Patch the `res_zoom_time.xy` field (bytes 0..8) of a packed engine push with
/// the given target size, leaving zoom/time/pan/params intact.
fn push_with_res(push: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = push.to_vec();
    if out.len() >= 8 {
        out[0..4].copy_from_slice(&(w as f32).to_le_bytes());
        out[4..8].copy_from_slice(&(h as f32).to_le_bytes());
    }
    out
}

fn begin(dev: &ash::Device, cmd: vk::CommandBuffer, view: vk::ImageView, w: u32, h: u32) {
    let attach = vk::RenderingAttachmentInfo::default()
        .image_view(view)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .clear_value(vk::ClearValue { color: vk::ClearColorValue { float32: [0.0; 4] } });
    let area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D { width: w, height: h },
    };
    unsafe {
        dev.cmd_begin_rendering(
            cmd,
            &vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(std::slice::from_ref(&attach)),
        );
        dev.cmd_set_viewport(cmd, 0, &[vk::Viewport {
            x: 0.0, y: 0.0, width: w as f32, height: h as f32, min_depth: 0.0, max_depth: 1.0,
        }]);
        dev.cmd_set_scissor(cmd, 0, &[area]);
    }
}

fn create_target(
    dev: &VulkanDevice,
    mem: &vk::PhysicalDeviceMemoryProperties,
    format: vk::Format,
    w: u32,
    h: u32,
) -> Result<TargetImg, VulkanError> {
    let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED;
    create_image(dev, mem, format, w, h, usage)
}

/// Allocate a 2D image + view. `usage` is what separates a render target
/// (`COLOR_ATTACHMENT | SAMPLED`) from an uploaded texture
/// (`TRANSFER_DST | SAMPLED`); everything else about the two is identical, which
/// is why they share this and not two near-copies.
fn create_image(
    dev: &VulkanDevice,
    mem: &vk::PhysicalDeviceMemoryProperties,
    format: vk::Format,
    w: u32,
    h: u32,
    usage: vk::ImageUsageFlags,
) -> Result<TargetImg, VulkanError> {
    let device = &dev.device;
    let image = unsafe {
        device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("graph target image: {e}")))?
    };
    let req = unsafe { device.get_image_memory_requirements(image) };
    let mem_idx = device_local(mem, req.memory_type_bits)
        .ok_or_else(|| VulkanError::Vk("graph target: no device-local memory".into()))?;
    let memory = unsafe {
        device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(mem_idx),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("graph target memory: {e}")))?
    };
    unsafe {
        device
            .bind_image_memory(image, memory, 0)
            .map_err(|e| VulkanError::Vk(format!("graph target bind: {e}")))?
    };
    let view = unsafe {
        device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(range()),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("graph target view: {e}")))?
    };
    Ok(TargetImg { image, memory, view, w, h, format })
}

/// Max world entries carried in the UBO
/// (`struct Windows { count; rects[N]; srcs[N]; meta[N] }`).
///
/// This is the declared length of the WGSL uniform arrays, which the language
/// requires to be a compile-time constant — so every bundle that declares
/// `struct Windows` must use the SAME number, and `tests/wgsl_abi.rs` checks that
/// the shipped examples do. Raised from 64 when the set widened from client
/// windows to the whole world band; at 256 the UBO is 12 KiB, comfortably inside
/// the 16 KiB `maxUniformBufferRange` every Vulkan implementation guarantees.
/// Going unbounded means a storage buffer plus a variable-count descriptor array,
/// which is an ABI break for every one of those bundles.
pub const MAX_WORLD_ENTRIES: usize = 256;
// The shader header is `count: u32` followed by `_pad: vec3<u32>`. `vec3<u32>`
// has SIXTEEN-byte alignment, so the pad starts at offset 16 (not 4) and the
// `array<vec4<f32>>` after it starts at 32 — the header is 32 bytes, not 16.
// Writing the arrays at 16 made every shader read `rects[i]` as `rects[i+1]`
// while the texture array (a descriptor array, unaffected) stayed correct: each
// window was painted with the NEXT window's rectangle and its OWN texture, which
// on screen reads as every window showing another window's content.
const WORLD_RECTS_OFFSET: u64 = 32;
const WORLD_SRCS_OFFSET: u64 = WORLD_RECTS_OFFSET + (MAX_WORLD_ENTRIES * 16) as u64;
const WORLD_META_OFFSET: u64 = WORLD_SRCS_OFFSET + (MAX_WORLD_ENTRIES * 16) as u64;
const WINDOW_UBO_BYTES: u64 = WORLD_META_OFFSET + (MAX_WORLD_ENTRIES * 16) as u64;

// The `Times` block (`@group(1) @binding(2)`), in the SAME buffer at an offset
// rather than in one of its own. Two ranges of one allocation is one map and one
// lifetime; it also keeps the geometry block at exactly the size every shipped
// `struct Windows` declares, which a fourth array inside it could not — three
// arrays plus a fourth is 16416 bytes, past the 16 KiB `maxUniformBufferRange`
// every implementation guarantees, so it would have meant dropping the entry
// count and re-writing every bundle in the tree.
//
// The offset is ALIGNED UP to 256, the largest `minUniformBufferOffsetAlignment`
// in practice. It is not aligned by construction: the geometry block is
// 32 + 3 * 256 * 16 = 12320 bytes, which is 32 past a multiple of 256, and binding
// a uniform buffer at an unaligned offset is a validation error on every device
// that reports the usual limit. Rounding up wastes 224 bytes once.
pub const TIMES_OFFSET: u64 = WINDOW_UBO_BYTES.next_multiple_of(256);
const TIMES_LIFE_OFFSET: u64 = TIMES_OFFSET + 32;
const TIMES_STATE_OFFSET: u64 = TIMES_LIFE_OFFSET + (MAX_WORLD_ENTRIES * 16) as u64;
const TIMES_DRAG_OFFSET: u64 = TIMES_STATE_OFFSET + (MAX_WORLD_ENTRIES * 16) as u64;
const TIMES_BYTES: u64 = 32 + 3 * (MAX_WORLD_ENTRIES * 16) as u64;

// The `Pointer` block (`@group(1) @binding(3)`), the seat-wide half. Two vec4s —
// tiny, but it needs its own binding rather than a corner of one of the blocks
// above: those are indexed by drawable and this is not, and squeezing a frame-wide
// value into a per-entry array's header is how a reader ends up multiplying it by
// an index. Same buffer again, aligned the same way.
const POINTER_OFFSET: u64 = (TIMES_OFFSET + TIMES_BYTES).next_multiple_of(256);
const POINTER_BYTES: u64 = 32;
const WINDOW_BUFFER_BYTES: u64 = POINTER_OFFSET + POINTER_BYTES;

/// Max entries in the bindless world-texture array (`@group(1) @binding(1)`),
/// index-aligned with the rects UBO. Partially bound: only the live entries are
/// written each frame, so unused slots cost nothing.
pub const MAX_WINDOW_TEXTURES: usize = MAX_WORLD_ENTRIES;

fn host_visible(mem: &vk::PhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    (0..mem.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && mem.memory_types[i as usize].property_flags.contains(
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )
    })
}

/// Owns the intermediate targets + per-pass `EffectPass` cache for one pipeline,
/// plus the shared window-rects UBO (`@group(1)`) built on demand.
#[derive(Default)]
pub struct GraphExec {
    targets: Vec<TargetImg>,
    /// The second image of each PERSISTENT target, `None` for ordinary ones.
    ///
    /// Index-aligned with `targets`, so a target index resolves into both without
    /// a map. Which of the pair is written and which is sampled alternates with
    /// the frame — see [`GraphExec::write`] / [`GraphExec::read`].
    mirrors: Vec<Option<TargetImg>>,
    /// One buffer per declared storage, in `@group(2)` binding order, plus the
    /// layout and set that bind them. Empty for the bundles — nearly all of them —
    /// that declare none, so the allocation and the descriptor pool never happen.
    store_bufs: Vec<(vk::Buffer, vk::DeviceMemory, u64)>,
    store_layout: vk::DescriptorSetLayout,
    store_pool: vk::DescriptorPool,
    store_set: vk::DescriptorSet,
    /// The bundle's uploaded textures, in `TEXTURE` sentinel order. Empty for
    /// nearly every bundle, which is what keeps the staging allocation and the
    /// copy out of existence.
    images: Vec<TargetImg>,
    /// Host-visible staging for [`GraphExec::images`], filled in
    /// [`GraphExec::prepare`] and copied on the first recorded frame — the only
    /// point a command buffer exists, exactly as for `clear_pristine`.
    staging: Vec<(vk::Buffer, vk::DeviceMemory)>,
    /// Frames to hold `staging` before freeing it.
    ///
    /// The copy is recorded into a frame's command buffer, so the buffers must
    /// outlive that submission — and this executor never waits for one. Four
    /// `prepare` calls is past any in-flight depth on either path that runs it
    /// (the compositor submits one frame at a time; the worker buffers at most
    /// three slots), and `prepare` runs every frame, so the countdown is in the
    /// same units the risk is.
    staging_ttl: u32,
    /// Both images of every persistent target are undefined until something writes
    /// them, and the FIRST frame samples one that nothing has. Zeroed once, on the
    /// first recorded frame after allocation, because that is the first point a
    /// command buffer exists.
    pristine: std::cell::Cell<bool>,
    passes: HashMap<(u64, vk::Format), EffectPass>,
    pipeline_id: u64,
    extent: (u32, u32),
    swap_format: vk::Format,
    win_buf: vk::Buffer,
    win_mem: vk::DeviceMemory,
    win_layout: vk::DescriptorSetLayout,
    win_pool: vk::DescriptorPool,
    win_set: vk::DescriptorSet,
}

impl GraphExec {
    /// Lazily create the window-rects UBO (buffer + descriptor set + layout). The
    /// layout is handed to `EffectPass::create` for `world_set()` passes; the set
    /// is bound at draw time. No-op once built.
    fn ensure_window_ubo(
        &mut self,
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<(), VulkanError> {
        if self.win_buf != vk::Buffer::null() {
            return Ok(());
        }
        let d = &dev.device;
        self.win_buf = unsafe {
            d.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(WINDOW_BUFFER_BYTES)
                    .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("window ubo buffer: {e}")))?
        };
        let req = unsafe { d.get_buffer_memory_requirements(self.win_buf) };
        let idx = host_visible(mem, req.memory_type_bits)
            .ok_or_else(|| VulkanError::Vk("window ubo: no host-visible memory".into()))?;
        self.win_mem = unsafe {
            d.allocate_memory(
                &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(idx),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("window ubo memory: {e}")))?
        };
        unsafe {
            d.bind_buffer_memory(self.win_buf, self.win_mem, 0)
                .map_err(|e| VulkanError::Vk(format!("window ubo bind: {e}")))?;
        }
        // set 1 (@group(1)): binding 0 = window-rects UBO (always). binding 1 = a
        // partially-bound bindless window-texture array, ONLY when the device
        // enabled descriptor indexing (else window-textures bundles were gated out
        // by the producer, so no shader references binding 1).
        let di = dev.descriptor_indexing;
        let mut bindings = vec![vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        if di {
            bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(MAX_WINDOW_TEXTURES as u32)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            );
        }
        // Binding 2 is the `Times` block. Declared UNCONDITIONALLY, unlike the
        // texture array: that one is gated on a device FEATURE, so a device without
        // it could not have the binding at all, whereas this is gated on a bundle
        // REQUIREMENT, which changes at runtime. A layout that changed with the
        // loaded bundle would have to be torn down and rebuilt on every selection,
        // and every cached `EffectPass` with it. The descriptor is written once and
        // costs 8 KiB of the same host-visible allocation; what `window_times`
        // actually gates is the per-frame tracking, gather and upload, which is
        // where the cost is.
        bindings.push(
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        );
        bindings.push(
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        );
        // Per-binding flags: the texture array is PARTIALLY_BOUND (only live
        // windows written). Length must match `bindings`.
        let flags = [
            vk::DescriptorBindingFlags::empty(),
            vk::DescriptorBindingFlags::PARTIALLY_BOUND,
            vk::DescriptorBindingFlags::empty(),
            vk::DescriptorBindingFlags::empty(),
        ];
        let mut flags_info = vk::DescriptorSetLayoutBindingFlagsCreateInfo::default()
            .binding_flags(&flags[..bindings.len()]);
        let mut layout_info =
            vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        if di {
            layout_info = layout_info.push_next(&mut flags_info);
        }
        self.win_layout = unsafe {
            d.create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| VulkanError::Vk(format!("window set layout: {e}")))?
        };
        let mut pool_sizes = vec![vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(3)];
        if di {
            pool_sizes.push(
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(MAX_WINDOW_TEXTURES as u32),
            );
        }
        self.win_pool = unsafe {
            d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&pool_sizes),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("window pool: {e}")))?
        };
        self.win_set = unsafe {
            d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.win_pool)
                    .set_layouts(std::slice::from_ref(&self.win_layout)),
            )
            .map_err(|e| VulkanError::Vk(format!("window ubo set: {e}")))?[0]
        };
        let info = vk::DescriptorBufferInfo::default()
            .buffer(self.win_buf)
            .offset(0)
            .range(WINDOW_UBO_BYTES);
        let times = vk::DescriptorBufferInfo::default()
            .buffer(self.win_buf)
            .offset(TIMES_OFFSET)
            .range(TIMES_BYTES);
        let pointer = vk::DescriptorBufferInfo::default()
            .buffer(self.win_buf)
            .offset(POINTER_OFFSET)
            .range(POINTER_BYTES);
        unsafe {
            d.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(self.win_set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(std::slice::from_ref(&info)),
                    vk::WriteDescriptorSet::default()
                        .dst_set(self.win_set)
                        .dst_binding(2)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(std::slice::from_ref(&times)),
                    vk::WriteDescriptorSet::default()
                        .dst_set(self.win_set)
                        .dst_binding(3)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(std::slice::from_ref(&pointer)),
                ],
                &[],
            );
        }
        Ok(())
    }

    /// Upload the current world entries into the UBO: screen-UV rects
    /// `[x, y, w, h]`, index-aligned texture src crops (texture UV `[x, y, w, h]`)
    /// and per-entry `meta` = `[kind, alpha, 0, 0]`. No-op until the UBO exists (a
    /// pipeline requiring the world set has run).
    ///
    /// All three slices must be lockstep — same length, same op order — because a
    /// shader reads `rects[i]`, `srcs[i]`, `meta[i]` and `win_tex[i]` as one
    /// drawable. `submit.rs` builds them in a single pass for exactly that reason.
    pub fn set_world_entries(
        &self,
        dev: &VulkanDevice,
        rects: &[[f32; 4]],
        srcs: &[[f32; 4]],
        meta: &[[f32; 4]],
    ) {
        if self.win_mem == vk::DeviceMemory::null() {
            return;
        }
        let n = rects.len().min(srcs.len()).min(meta.len()).min(MAX_WORLD_ENTRIES);
        unsafe {
            let ptr = match dev.device.map_memory(
                self.win_mem, 0, WINDOW_UBO_BYTES, vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => p as *mut u8,
                Err(_) => return,
            };
            (ptr as *mut u32).write_unaligned(n as u32);
            let write = |off: u64, src: &[[f32; 4]]| {
                let dst = ptr.add(off as usize) as *mut [f32; 4];
                for (i, v) in src.iter().take(n).enumerate() {
                    dst.add(i).write_unaligned(*v);
                }
            };
            write(WORLD_RECTS_OFFSET, rects);
            write(WORLD_SRCS_OFFSET, srcs);
            write(WORLD_META_OFFSET, meta);
            dev.device.unmap_memory(self.win_mem);
        }
    }

    /// Upload the per-entry timestamps into the `Times` block, index-aligned with
    /// the geometry.
    ///
    /// A NO-OP when the slices are empty, which is exactly the case where the
    /// bundle did not declare `window_times`: the block keeps whatever it last
    /// held and no shader is reading it. That is the requirement's whole effect on
    /// this side — nothing is mapped, nothing is written, nothing is walked.
    pub fn set_world_times(
        &self,
        dev: &VulkanDevice,
        life: &[[f32; 4]],
        state: &[[f32; 4]],
        drag: &[[f32; 4]],
    ) {
        if self.win_mem == vk::DeviceMemory::null() || life.is_empty() {
            return;
        }
        let n = life.len().min(state.len()).min(drag.len()).min(MAX_WORLD_ENTRIES);
        unsafe {
            let ptr = match dev.device.map_memory(
                self.win_mem, 0, WINDOW_BUFFER_BYTES, vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => p as *mut u8,
                Err(_) => return,
            };
            (ptr.add(TIMES_OFFSET as usize) as *mut u32).write_unaligned(n as u32);
            let write = |off: u64, src: &[[f32; 4]]| {
                let dst = ptr.add(off as usize) as *mut [f32; 4];
                for (i, v) in src.iter().take(n).enumerate() {
                    dst.add(i).write_unaligned(*v);
                }
            };
            write(TIMES_LIFE_OFFSET, life);
            write(TIMES_STATE_OFFSET, state);
            write(TIMES_DRAG_OFFSET, drag);
            dev.device.unmap_memory(self.win_mem);
        }
    }

    /// Upload the seat-wide pointer block.
    ///
    /// Reads the published slot itself rather than taking the values, because both
    /// callers would otherwise read the same slot and pack it the same way — the
    /// duplication the `attrs` packer exists to prevent.
    pub fn set_pointer(&self, dev: &VulkanDevice) {
        if self.win_mem == vk::DeviceMemory::null() {
            return;
        }
        let rows = compositor_orchestration_seat_pointer_publish::publish::packed(
            compositor_pipeline_abi_clock_base::base::NEVER,
        );
        unsafe {
            let ptr = match dev.device.map_memory(
                self.win_mem, 0, WINDOW_BUFFER_BYTES, vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => p as *mut u8,
                Err(_) => return,
            };
            let dst = ptr.add(POINTER_OFFSET as usize) as *mut [f32; 4];
            dst.write_unaligned(rows[0]);
            dst.add(1).write_unaligned(rows[1]);
            dev.device.unmap_memory(self.win_mem);
        }
    }

    /// Write the current frame's window texture views into the bindless array at
    /// `@group(1) @binding(1)` (index-aligned with the rects). No-op unless the
    /// device enabled descriptor indexing and the window set exists. Safe to
    /// rewrite each frame under the synchronous submit path (device idle between
    /// frames); the array is partially bound so only the live windows are written.
    pub fn set_window_textures(&self, dev: &VulkanDevice, views: &[vk::ImageView]) {
        if !dev.descriptor_indexing || self.win_set == vk::DescriptorSet::null() {
            return;
        }
        let n = views.len().min(MAX_WINDOW_TEXTURES);
        if n == 0 {
            return;
        }
        let infos: Vec<vk::DescriptorImageInfo> = views
            .iter()
            .take(n)
            .map(|v| {
                vk::DescriptorImageInfo::default()
                    .image_view(*v)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect();
        unsafe {
            dev.device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(self.win_set)
                    .dst_binding(1)
                    .dst_array_element(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&infos)],
                &[],
            );
        }
    }

    /// The world-set descriptor for a pass, if it requires one and one exists.
    /// Both interfaces share the set — the textures are index-aligned with the
    /// rects, so a pass asking for either is handed the same binding.
    /// Allocate the declared storage buffers and the set that binds them.
    ///
    /// Device-local: these are written and read by the GPU and never touched by
    /// the CPU, so host-visible memory would trade bandwidth for nothing. That is
    /// also why they are zeroed with `cmd_fill_buffer` on the first recorded
    /// frame rather than by mapping them.
    fn build_storage(
        &mut self,
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        sizes: &[u64],
    ) -> Result<(), VulkanError> {
        if sizes.is_empty() {
            return Ok(());
        }
        let d = &dev.device;
        for &bytes in sizes {
            let buf = unsafe {
                d.create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(bytes)
                        .usage(
                            vk::BufferUsageFlags::STORAGE_BUFFER
                                | vk::BufferUsageFlags::TRANSFER_DST,
                        )
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("storage buffer: {e}")))?
            };
            let req = unsafe { d.get_buffer_memory_requirements(buf) };
            let idx = (0..mem.memory_type_count)
                .find(|&i| {
                    req.memory_type_bits & (1 << i) != 0
                        && mem.memory_types[i as usize]
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                })
                .ok_or_else(|| VulkanError::Vk("storage buffer: no device-local memory".into()))?;
            let memory = unsafe {
                d.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(idx),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("storage memory: {e}")))?
            };
            unsafe {
                d.bind_buffer_memory(buf, memory, 0)
                    .map_err(|e| VulkanError::Vk(format!("storage bind: {e}")))?;
            }
            self.store_bufs.push((buf, memory, bytes));
        }
        let bindings: Vec<vk::DescriptorSetLayoutBinding> = (0..sizes.len())
            .map(|i| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(i as u32)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            })
            .collect();
        self.store_layout = unsafe {
            d.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("storage set layout: {e}")))?
        };
        let sizes_pool = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(sizes.len() as u32)];
        self.store_pool = unsafe {
            d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&sizes_pool),
                None,
            )
            .map_err(|e| VulkanError::Vk(format!("storage pool: {e}")))?
        };
        self.store_set = unsafe {
            d.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.store_pool)
                    .set_layouts(std::slice::from_ref(&self.store_layout)),
            )
            .map_err(|e| VulkanError::Vk(format!("storage set: {e}")))?[0]
        };
        let infos: Vec<vk::DescriptorBufferInfo> = self
            .store_bufs
            .iter()
            .map(|&(buf, _, bytes)| {
                vk::DescriptorBufferInfo::default().buffer(buf).offset(0).range(bytes)
            })
            .collect();
        let writes: Vec<vk::WriteDescriptorSet> = infos
            .iter()
            .enumerate()
            .map(|(i, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(self.store_set)
                    .dst_binding(i as u32)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(std::slice::from_ref(info))
            })
            .collect();
        unsafe { d.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    /// Allocate one image per declared texture and stage its pixels.
    ///
    /// Nothing is copied here — a copy is a command and there is no command buffer
    /// at `prepare` time. The bytes land in host-visible staging now and the
    /// transfer is recorded on the first frame, beside the pristine clear, which
    /// is the same shape and the same reason.
    fn build_textures(
        &mut self,
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        textures: &[GraphTexture],
    ) -> Result<(), VulkanError> {
        if textures.is_empty() {
            return Ok(());
        }
        let d = &dev.device;
        for t in textures {
            // `_SRGB` vs `_UNORM` is the whole of the colour-space decision: the
            // sampler returns linear values for the first and raw ones for the
            // second, at no cost either way. See `manifest::Texture::srgb`.
            let format = match t.srgb {
                true => vk::Format::R8G8B8A8_SRGB,
                false => vk::Format::R8G8B8A8_UNORM,
            };
            let usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
            self.images.push(create_image(dev, mem, format, t.width, t.height, usage)?);

            let bytes = t.pixels.len() as u64;
            let buf = unsafe {
                d.create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(bytes.max(1))
                        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("texture staging buffer: {e}")))?
            };
            let req = unsafe { d.get_buffer_memory_requirements(buf) };
            let idx = host_visible(mem, req.memory_type_bits)
                .ok_or_else(|| VulkanError::Vk("texture staging: no host-visible memory".into()))?;
            let memory = unsafe {
                d.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(idx),
                    None,
                )
                .map_err(|e| VulkanError::Vk(format!("texture staging memory: {e}")))?
            };
            unsafe {
                d.bind_buffer_memory(buf, memory, 0)
                    .map_err(|e| VulkanError::Vk(format!("texture staging bind: {e}")))?;
                let ptr = d
                    .map_memory(memory, 0, bytes.max(1), vk::MemoryMapFlags::empty())
                    .map_err(|e| VulkanError::Vk(format!("texture staging map: {e}")))?
                    as *mut u8;
                std::ptr::copy_nonoverlapping(t.pixels.as_ptr(), ptr, t.pixels.len());
                d.unmap_memory(memory);
            }
            self.staging.push((buf, memory));
        }
        Ok(())
    }

    /// Record the staged pixel copies, once, on the first frame that has a command
    /// buffer. Each image ends in `SHADER_READ_ONLY_OPTIMAL` and is never written
    /// again, so this is the only barrier pair a texture ever needs.
    fn upload_textures(&self, dev: &VulkanDevice, cmd: vk::CommandBuffer) {
        for (img, &(buf, _)) in self.images.iter().zip(self.staging.iter()) {
            barrier(
                &dev.device, cmd, img.image,
                vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COPY,
                vk::AccessFlags2::empty(), vk::AccessFlags2::TRANSFER_WRITE,
            );
            let region = vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(vk::Extent3D { width: img.w, height: img.h, depth: 1 });
            unsafe {
                dev.device.cmd_copy_buffer_to_image(
                    cmd,
                    buf,
                    img.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    std::slice::from_ref(&region),
                );
            }
            barrier(
                &dev.device, cmd, img.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COPY, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::TRANSFER_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
            );
        }
    }

    /// Release staging once the frame that copied from it can no longer be in
    /// flight. See [`GraphExec::staging_ttl`].
    fn retire_staging(&mut self, dev: &VulkanDevice) {
        if self.staging.is_empty() || self.pristine.get() {
            return;
        }
        self.staging_ttl = self.staging_ttl.saturating_sub(1);
        if self.staging_ttl > 0 {
            return;
        }
        unsafe {
            for (buf, memory) in self.staging.drain(..) {
                dev.device.destroy_buffer(buf, None);
                dev.device.free_memory(memory, None);
            }
        }
    }

    /// The set bound at `@group(2)`, or `None` for a bundle with no storage.
    fn store_set_for(&self) -> Option<vk::DescriptorSet> {
        (self.store_set != vk::DescriptorSet::null()).then_some(self.store_set)
    }

    /// Storage is read-write across passes AND across frames, so every pass has
    /// to see what the previous one wrote. One buffer barrier per storage, after
    /// each pass — conservative (a pass that only reads pays for it too) and
    /// cheap, versus tracking per-pass access which the manifest does not declare.
    fn storage_barrier(&self, dev: &VulkanDevice, cmd: vk::CommandBuffer) {
        if self.store_bufs.is_empty() {
            return;
        }
        let b = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
            .dst_access_mask(
                vk::AccessFlags2::SHADER_STORAGE_READ | vk::AccessFlags2::SHADER_STORAGE_WRITE,
            );
        unsafe {
            dev.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(std::slice::from_ref(&b)),
            );
        }
    }

    /// Which image of target `i` this frame WRITES.
    ///
    /// Ordinary targets have one image and the parity is irrelevant. A persistent
    /// target alternates, so that the other image still holds last frame's result
    /// while this one is being overwritten.
    fn write(&self, i: usize, tick: u64) -> &TargetImg {
        match &self.mirrors[i] {
            Some(m) if tick % 2 == 1 => m,
            _ => &self.targets[i],
        }
    }

    /// Which image of target `i` a pass SAMPLES.
    ///
    /// The opposite of [`GraphExec::write`] for a persistent target — that is the
    /// whole feature: "read this target" means "read what was written last frame".
    /// For an ordinary target it is the same image the pass just wrote, which is
    /// the existing intermediate behaviour and must not change.
    fn read(&self, i: usize, tick: u64) -> &TargetImg {
        match &self.mirrors[i] {
            Some(m) if tick % 2 == 0 => m,
            _ => &self.targets[i],
        }
    }

    /// Zero both images of every persistent target, once.
    ///
    /// Without this the first frame samples memory nothing has written, which is
    /// undefined — in practice whatever the allocator last held, which reads as a
    /// flash of another window's pixels on a bundle whose whole point is to build
    /// its picture up gradually.
    fn clear_pristine(&self, dev: &VulkanDevice, cmd: vk::CommandBuffer) {
        if !self.pristine.replace(false) {
            return;
        }
        // The textures' one-time copy rides the same "first frame with a command
        // buffer" moment. It is not a clear, but it has exactly the same shape and
        // the same trigger, and giving it a second flag would mean two things that
        // must happen on the same frame being able to disagree about which one.
        self.upload_textures(dev, cmd);
        // Runs for a storage-only bundle too — `pristine` is about everything that
        // survives frames, not only the images.
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        // Storage buffers are undefined at allocation too, and a shader that
        // ACCUMULATES into one starts from whatever the allocator last held.
        for &(buf, _, bytes) in &self.store_bufs {
            unsafe { dev.device.cmd_fill_buffer(cmd, buf, 0, bytes, 0) };
        }
        if !self.store_bufs.is_empty() {
            let b = vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::CLEAR)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_access_mask(
                    vk::AccessFlags2::SHADER_STORAGE_READ | vk::AccessFlags2::SHADER_STORAGE_WRITE,
                );
            unsafe {
                dev.device.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().memory_barriers(std::slice::from_ref(&b)),
                );
            }
        }
        let zero = vk::ClearColorValue { float32: [0.0; 4] };
        for (i, m) in self.mirrors.iter().enumerate() {
            if m.is_none() {
                continue;
            }
            for t in [&self.targets[i], m.as_ref().expect("checked")] {
                barrier(
                    &dev.device, cmd, t.image,
                    vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::CLEAR,
                    vk::AccessFlags2::empty(), vk::AccessFlags2::TRANSFER_WRITE,
                );
                unsafe {
                    dev.device.cmd_clear_color_image(
                        cmd, t.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &zero,
                        std::slice::from_ref(&range),
                    );
                }
                barrier(
                    &dev.device, cmd, t.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::CLEAR, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::TRANSFER_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
                );
            }
        }
    }

    /// Set 1 is bound for a pass that wants ANY of its blocks: the geometry, the
    /// timestamps, or the pointer. `pointer()` is separate from `world_set()`
    /// because the pointer implies no drawables — a bundle that only wants the
    /// cursor still needs the set bound, and still pays for no collection.
    fn wants_set1(requires: Requires) -> bool {
        requires.world_set() || requires.pointer()
    }

    fn win_set_for(&self, requires: Requires) -> Option<vk::DescriptorSet> {
        (Self::wants_set1(requires) && self.win_set != vk::DescriptorSet::null())
            .then_some(self.win_set)
    }

    /// (Re)allocate targets and build the per-pass pipelines for `p`. Rebuilds
    /// when the pipeline id, output extent, or swapchain format changes.
    pub fn prepare(
        &mut self,
        dev: &VulkanDevice,
        mem: &vk::PhysicalDeviceMemoryProperties,
        cache: vk::PipelineCache,
        p: &GraphPipeline,
        extent: (u32, u32),
        swap_format: vk::Format,
    ) -> Result<(), VulkanError> {
        let changed =
            self.pipeline_id != p.id || self.extent != extent || self.swap_format != swap_format;
        if changed {
            self.teardown(dev);
            self.pipeline_id = p.id;
            self.extent = extent;
            self.swap_format = swap_format;
            for &(fmt, scale, persist) in &p.targets {
                let w = ((extent.0 as f32 * scale).round() as u32).max(1);
                let h = ((extent.1 as f32 * scale).round() as u32).max(1);
                self.targets.push(create_target(dev, mem, fmt.vk(), w, h)?);
                // A persistent target is a PAIR. The alternative — one image with
                // a preserving load-op — cannot work here: a pass would be
                // sampling the image it is rendering to, which is a feedback loop
                // Vulkan forbids without an extension. Alternating costs one extra
                // image and nothing per frame, because a fullscreen pass writes
                // every pixel of its output regardless.
                self.mirrors.push(match persist {
                    true => Some(create_target(dev, mem, fmt.vk(), w, h)?),
                    false => None,
                });
            }
            self.pristine.set(true);
            self.build_storage(dev, mem, &p.storage)?;
            self.build_textures(dev, mem, &p.textures)?;
            self.staging_ttl = 4;
        }
        self.retire_staging(dev);
        if Self::wants_set1(p.requires) {
            self.ensure_window_ubo(dev, mem)?;
        }
        for pass in p.passes.iter().chain(p.after.iter()) {
            let fmt = match pass.output {
                GraphOutput::Target(i) => self.targets[i].format,
                GraphOutput::Swapchain => swap_format,
            };
            let key = (pass.id, fmt);
            if !self.passes.contains_key(&key) {
                let window_ubo = Self::wants_set1(pass.requires).then_some(self.win_layout);
                let ep = EffectPass::create(
                    dev,
                    cache,
                    fmt,
                    pass.inputs.len() as u32,
                    &pass.spv,
                    pass.vert_spv.as_deref(),
                    &pass.vert_entry,
                    &pass.frag_entry,
                    pass.push.len().max(16) as u32,
                    window_ubo,
                    (!p.storage.is_empty()).then_some(self.store_layout),
                )?;
                self.passes.insert(key, ep);
            }
        }
        Ok(())
    }

    /// Run every intermediate (`Target`-output) pass into its target, OUTSIDE the
    /// main composite pass (call from the `pre` closure). Each target is left in
    /// `SHADER_READ_ONLY_OPTIMAL` for the passes that sample it.
    /// `tick` drives per-pass `cadence`: a pass with cadence N is recorded only
    /// when `tick % N == 0`. Off-tick it is simply not recorded, and its target
    /// keeps the previous run's contents — the targets are allocated once, not per
    /// frame, so "hold" needs no extra storage and no copy. The `output` pass has
    /// no cadence (rejected by `plan()`), so a frame always has a picture.
    pub fn record_intermediates(
        &self,
        dev: &VulkanDevice,
        cmd: vk::CommandBuffer,
        p: &GraphPipeline,
        tick: u64,
    ) {
        for ep in self.passes.values() {
            ep.begin_frame(dev);
        }
        self.clear_pristine(dev, cmd);
        for pass in &p.passes {
            let GraphOutput::Target(ti) = pass.output else { continue };
            if tick % pass.cadence.max(1) as u64 != 0 {
                continue;
            }
            let t = self.write(ti, tick);
            let ep = &self.passes[&(pass.id, t.format)];
            // Before-content passes cannot read `content`/`history` (validated).
            let Ok(set) = self.input_set(dev, ep, &pass.inputs, None, None, None, tick) else { continue };
            barrier(
                &dev.device, cmd, t.image,
                vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            );
            begin(&dev.device, cmd, t.view, t.w, t.h);
            ep.draw(dev, cmd, set, self.win_set_for(pass.requires), self.store_set_for(), &push_with_res(&pass.push, t.w, t.h));
            unsafe { dev.device.cmd_end_rendering(cmd) };
            barrier(
                &dev.device, cmd, t.image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
            );
            self.storage_barrier(dev, cmd);
        }
    }

    /// Draw the single before-content `Swapchain`-output pass INTO the current pass
    /// (call from the `compose` closure). No begin/end rendering — it draws into
    /// the already-open swapchain attachment, at the background slot.
    pub fn draw_output(&self, dev: &VulkanDevice, cmd: vk::CommandBuffer, p: &GraphPipeline, tick: u64) {
        let Some(pass) = p.passes.iter().find(|p| p.output == GraphOutput::Swapchain) else {
            return;
        };
        let Some(ep) = self.passes.get(&(pass.id, self.swap_format)) else {
            return;
        };
        if let Ok(set) = self.input_set(dev, ep, &pass.inputs, None, None, None, tick) {
            ep.draw(dev, cmd, set, self.win_set_for(pass.requires), self.store_set_for(), &push_with_res(&pass.push, self.extent.0, self.extent.1));
        }
    }

    fn input_set(
        &self,
        dev: &VulkanDevice,
        ep: &EffectPass,
        inputs: &[usize],
        content: Option<vk::ImageView>,
        history: Option<vk::ImageView>,
        windows: Option<vk::ImageView>,
        tick: u64,
    ) -> Result<Option<vk::DescriptorSet>, VulkanError> {
        // A pass with no inputs still needs set 0 when it binds window textures —
        // that is where its sampler lives.
        if inputs.is_empty() && !ep.has_input_set() {
            return Ok(None);
        }
        let views: Vec<vk::ImageView> = inputs
            .iter()
            .map(|&i| match i {
                CONTENT => content.unwrap_or_default(),
                HISTORY => history.unwrap_or_default(),
                WINDOWS => windows.unwrap_or_default(),
                // A range test, so it must come after the three equalities above.
                // Out of range cannot happen — the loader resolved these indices
                // against the same list — and yields a null view rather than a
                // panic if the two ever do disagree.
                i if i >= TEXTURE => self
                    .images
                    .get(i - TEXTURE)
                    .map(|t| t.view)
                    .unwrap_or_default(),
                // THE persistence seam: for a persistent target this is the image
                // the previous frame wrote, not the one this frame is writing.
                _ => self.read(i, tick).view,
            })
            .collect();
        ep.input_set(dev, &views).map(Some)
    }

    /// Run every after-content intermediate (`Target`-output) pass into its
    /// target, sampling `content` (and any before-content targets), OUTSIDE the
    /// swapchain render pass. Called once the scene is composited into `content`.
    pub fn record_after_intermediates(
        &self,
        dev: &VulkanDevice,
        cmd: vk::CommandBuffer,
        p: &GraphPipeline,
        content: vk::ImageView,
        history: vk::ImageView,
        windows: vk::ImageView,
        tick: u64,
    ) {
        self.clear_pristine(dev, cmd);
        for pass in &p.after {
            let GraphOutput::Target(ti) = pass.output else { continue };
            if tick % pass.cadence.max(1) as u64 != 0 {
                continue;
            }
            let t = self.write(ti, tick);
            let ep = &self.passes[&(pass.id, t.format)];
            let Ok(set) = self.input_set(dev, ep, &pass.inputs, Some(content), Some(history), Some(windows), tick) else { continue };
            barrier(
                &dev.device, cmd, t.image,
                vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::TOP_OF_PIPE, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::empty(), vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            );
            begin(&dev.device, cmd, t.view, t.w, t.h);
            ep.draw(dev, cmd, set, self.win_set_for(pass.requires), self.store_set_for(), &push_with_res(&pass.push, t.w, t.h));
            unsafe { dev.device.cmd_end_rendering(cmd) };
            barrier(
                &dev.device, cmd, t.image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT, vk::PipelineStageFlags2::FRAGMENT_SHADER,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE, vk::AccessFlags2::SHADER_SAMPLED_READ,
            );
            self.storage_barrier(dev, cmd);
        }
    }

    /// Draw the after-content `Swapchain`-output pass into the current swapchain
    /// render pass, sampling `content` (+ targets). This is the final image.
    pub fn draw_after_output(
        &self,
        dev: &VulkanDevice,
        cmd: vk::CommandBuffer,
        p: &GraphPipeline,
        content: vk::ImageView,
        history: vk::ImageView,
        windows: vk::ImageView,
        tick: u64,
    ) {
        let Some(pass) = p.after.iter().find(|p| p.output == GraphOutput::Swapchain) else {
            return;
        };
        let Some(ep) = self.passes.get(&(pass.id, self.swap_format)) else {
            return;
        };
        if let Ok(set) = self.input_set(dev, ep, &pass.inputs, Some(content), Some(history), Some(windows), tick) {
            ep.draw(dev, cmd, set, self.win_set_for(pass.requires), self.store_set_for(), &push_with_res(&pass.push, self.extent.0, self.extent.1));
        }
    }

    /// Drop the intermediates because no bundle is loaded any more, keeping the
    /// executor itself reusable. Returns whether anything was actually freed.
    ///
    /// [`Self::prepare`] only ever releases as a side effect of building the NEXT
    /// pipeline, so an executor whose bundle is unloaded — switching from a
    /// multipass bundle back to the built-in parallax — keeps every target it
    /// last built, at output resolution, for the rest of the session. Same shape
    /// as the `ContentPath` layers: allocation was driven by the requirement
    /// arriving and nothing was driven by it going away.
    ///
    /// `extent` is cleared as well as `pipeline_id`, so the next `prepare`
    /// rebuilds even in the (unlikely) case that a real pipeline id is 0 — the
    /// same value a never-prepared executor starts at.
    pub fn release(&mut self, dev: &VulkanDevice) -> bool {
        if self.targets.is_empty() && self.mirrors.is_empty() && self.images.is_empty() {
            return false;
        }
        self.teardown(dev);
        self.pipeline_id = 0;
        self.extent = (0, 0);
        true
    }

    fn teardown(&mut self, dev: &VulkanDevice) {
        for t in self.targets.drain(..) {
            t.destroy(dev);
        }
        for m in self.mirrors.drain(..).flatten() {
            m.destroy(dev);
        }
        for t in self.images.drain(..) {
            t.destroy(dev);
        }
        let d = &dev.device;
        unsafe {
            // Staging that never reached its TTL — the bundle changed within a few
            // frames of being selected, which is exactly what clicking through the
            // picker does.
            for (buf, memory) in self.staging.drain(..) {
                d.destroy_buffer(buf, None);
                d.free_memory(memory, None);
            }
            for (buf, memory, _) in self.store_bufs.drain(..) {
                d.destroy_buffer(buf, None);
                d.free_memory(memory, None);
            }
            if self.store_pool != vk::DescriptorPool::null() {
                d.destroy_descriptor_pool(self.store_pool, None);
                self.store_pool = vk::DescriptorPool::null();
            }
            if self.store_layout != vk::DescriptorSetLayout::null() {
                d.destroy_descriptor_set_layout(self.store_layout, None);
                self.store_layout = vk::DescriptorSetLayout::null();
            }
        }
        self.store_set = vk::DescriptorSet::null();
        for (_, ep) in self.passes.drain() {
            ep.destroy(dev);
        }
    }

    pub fn destroy(&mut self, dev: &VulkanDevice) {
        self.teardown(dev);
        let d = &dev.device;
        unsafe {
            if self.win_pool != vk::DescriptorPool::null() {
                d.destroy_descriptor_pool(self.win_pool, None);
            }
            if self.win_layout != vk::DescriptorSetLayout::null() {
                d.destroy_descriptor_set_layout(self.win_layout, None);
            }
            if self.win_buf != vk::Buffer::null() {
                d.destroy_buffer(self.win_buf, None);
            }
            if self.win_mem != vk::DeviceMemory::null() {
                d.free_memory(self.win_mem, None);
            }
        }
    }
}
