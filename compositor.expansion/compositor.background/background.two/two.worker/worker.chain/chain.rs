//! Record and submit a whole multipass graph into one slot.
//!
//! The single-pass sibling is `worker.render`. This runs the SAME executor the
//! compositor uses (`renderer.graph::GraphExec`) against the worker's own
//! `VkDevice`, so a background graph costs one implementation, not two.
//!
//! Only graphs the placement verdict calls `Offload::Whole` reach here
//! (`shader.place`): every pass is `before-content`, declares no engine `needs`,
//! and the band ends by writing `output`. So there is no window set to bind, no
//! `content` to sample, and nothing to hand back to the compositor except the
//! finished image — which is exactly why this stage needs no cross-device sync.
//!
//! Like `worker.render` it does NOT wait for completion: the caller owns that,
//! because whether it waits now or one frame later is the pipelining choice. What
//! the caller must not do is publish before the fence signals.

use ash::vk;
use compositor_pipeline_abi_worldset_base::base::Own;
use compositor_background_two_worker_device::device::Device;
use compositor_background_two_worker_history::history::History;
use compositor_background_two_worker_target::target::Target;
use compositor_pipeline_execute_graph_base::graph::{GraphExec, GraphPipeline};

/// The post-composite inputs a stage-4 band reads. Grouped because they travel
/// together and are meaningless apart: all three describe the same frame.
pub struct After<'a> {
    /// The compositor's composited world band — background and windows.
    pub content: vk::ImageView,
    /// The engine's own window layer, or null when no pass names it.
    pub windows: vk::ImageView,
    /// This pane's derived previous-pass band, or `None` when no pass names it.
    pub history: &'a mut Option<History>,
}

/// Build/refresh `graph` for `gp`, then record every pass into `target`.
///
/// The intermediates live on the worker device and are sized from `extent`, so
/// each pane needs its own `GraphExec` — panes differ in size, and a shared one
/// would rebuild every time the loop moved between them.
pub fn render_chain(
    d: &Device,
    graph: &mut GraphExec,
    imports: &mut compositor_background_two_worker_import::import::Imports,
    target: &Target,
    gp: &GraphPipeline,
    extent: (u32, u32),
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    tick: u64,
    // `warpmap`: this pane's GPU warp-map producer. Lives here because a
    // fully-offloaded bundle never reaches the compositor's — the graph runs on
    // THIS device, so the grid must be rendered on it too. The grid is plain host
    // floats, so publishing it across devices costs nothing.
    warpmap: &mut Option<compositor_pipeline_execute_warpmap_base::base::WarpMap>,
    // This world's drawables, carried on the request. The worker cannot reach a
    // world's storage, so the set travels to it rather than being fetched.
    world_set: Option<&compositor_pipeline_abi_worldset_base::base::WorldSet>,
    // `Some` = stage 4: run the AFTER band over the compositor's composited band
    // rather than the before band. `None` = the ordinary background case.
    mut after: Option<After<'_>>,
// `Ok(None)` = this pass says nothing about the grid; `Ok(Some(None))` = withdraw
// it. The two are different answers and collapsing them would have a pass that
// merely bailed early wipe a correction that is still valid.
) -> Result<Option<Option<std::sync::Arc<Vec<[f32; 2]>>>>, String> {
    // Memory properties for the intermediate allocations. Owned (Copy), so the
    // borrow of `phd` ends before `graph` is borrowed mutably.
    let mem = unsafe {
        d.dev
            .instance
            .get_physical_device_memory_properties(d.phd.handle())
    };
    // No pipeline cache on the worker device: its pass set is tiny and stable,
    // and a cache would be one more object to keep in step with the compositor's.
    graph
        .prepare(&d.dev, &mem, vk::PipelineCache::null(), gp, extent, target.format)
        .map_err(|e| format!("worker graph prepare: {e:?}"))?;

    // Bind this frame's window geometry, if any pass asked for it. Read from the
    // shared slot the compositor publishes: numbers only, so nothing is imported
    // and nothing has to be kept alive across devices.
    // Imported window buffers to acquire into this frame's command buffer. An
    // imported image arrives in `UNDEFINED` layout and the shader samples it as
    // `SHADER_READ_ONLY_OPTIMAL` (`graph::set_window_textures` writes that layout
    // into the descriptor), so it MUST be transitioned or the sample is undefined.
    // The compositor does the same for its own imports and re-does it every frame
    // for cache-served ones (`record_pending_acquires`) — a cached import is not a
    // transitioned one, because its contents arrive from outside again each frame.
    let mut acquires: Vec<compositor_background_two_worker_import::import::Acquired> = Vec::new();
    // Independent of the world set: a bundle can want the cursor and no drawables.
    if gp.requires.pointer() {
        graph.set_pointer(&d.dev);
    }
    let wants_rects = gp.requires.world_set();
    let wants_tex = gp.requires.textures();
    if wants_rects || wants_tex {
        let Some(w) = world_set else { return Ok(None) };
        // The SAME filter the compositor applies (`WorldSet::owned`), so a bundle
        // sees one array whichever side runs it. Offloaded, this is the only place
        // it happens — there is no pipeline op in the compositor's frame to carry
        // the mode, which is why `own()` is read from the shared slot.
        let owned = if gp.requires.whole_band() { w.owned(Own::World) } else { w.owned(gp.owns) };
        let rects: Vec<[f32; 4]> = owned.iter().map(|&i| w.rects[i]).collect();
        let srcs: Vec<[f32; 4]> = owned.iter().map(|&i| w.srcs[i]).collect();
        // The SAME packer the compositor's own composite uses. This array had a
        // literal here and a matching literal there; a descriptor added to one
        // alone is an effect that works inline and silently does nothing offloaded.
        let meta: Vec<[f32; 4]> = owned
            .iter()
            .map(|&i| {
                compositor_pipeline_abi_descriptor_base::base::attrs(
                    w.kinds[i] as u8,
                    w.alphas[i],
                    w.flags[i],
                )
            })
            .collect();
        graph.set_world_entries(&d.dev, &rects, &srcs, &meta);
        // The same split the compositor's `Collected::bind` does, over the same
        // published rows — the timestamps are per-ENTRY and must survive the
        // ownership filter with their entries, or they describe other windows.
        if gp.requires.times() {
            let life: Vec<[f32; 4]> = owned
                .iter()
                .filter(|&&i| i < w.times.len())
                .map(|&i| [w.times[i][0], w.times[i][1], w.times[i][2], w.times[i][3]])
                .collect();
            let state: Vec<[f32; 4]> = owned
                .iter()
                .filter(|&&i| i < w.times.len())
                .map(|&i| [w.times[i][4], w.times[i][5], w.times[i][6], w.times[i][7]])
                .collect();
            let drag: Vec<[f32; 4]> = owned
                .iter()
                .filter(|&&i| i < w.times.len())
                .map(|&i| [w.times[i][8], w.times[i][9], w.times[i][10], w.times[i][11]])
                .collect();
            graph.set_world_times(&d.dev, &life, &state, &drag);
        }
        if wants_tex {
            // Import each window's client buffer into THIS device. The importer
            // dups the fd, so these stay valid independently of the compositor's
            // own imports and of the client releasing the buffer.
            //
            // Both client types import here: a dmabuf directly, a SHM surface via
            // the OPAQUE_FD share of the image the compositor uploaded into. An
            // entry that cannot be imported at all voids the whole set rather than
            // binding a partial array, which would leave the shader sampling
            // whichever stale descriptor was last written into that slot.
            let mut views = Vec::with_capacity(owned.len());
            for src in owned.iter().map(|&i| &w.sources[i]) {
                match imports.view(d, src) {
                    Ok(a) => {
                        views.push(a.view);
                        acquires.push(a);
                    }
                    Err(_) => {
                        views.clear();
                        acquires.clear();
                        break;
                    }
                }
            }
            graph.set_window_textures(&d.dev, &views);
        }
    }

    d.acquire(fence)?;
    // `acquire` waited on this fence, so the PREVIOUS submit on this pane is
    // complete — the same proof the compositor gets from its frame fence, and the
    // only point at which mapping the staging buffer is a memcpy rather than a
    // wait. Read the grid recorded last pass before re-recording over it.
    // Handed back to `serve_pane`, which puts it in this pane's readback slot —
    // the compositor's pointer warp reads it from there. Returned rather than
    // published: the compositor's own map produces one too, and one shared slot
    // could not say whose grid it held.
    let produced = compositor_pipeline_execute_warpmap_base::base::WarpMap::publish(warpmap, &d.dev);
    // Build or drop the producer to match what this bundle asks for — the SAME
    // function the compositor calls, on this device.
    let invalidated = compositor_pipeline_execute_warpmap_base::base::WarpMap::sync(
        &d.dev, &mem, vk::PipelineCache::null(), gp.warp_map.as_ref(), warpmap, "worker's",
    )
    .map_err(|e| format!("worker warp map: {e:?}"))?;
    // `None` withdraws: a grid outliving its bundle displaces the pointer by an
    // effect that is no longer on screen.
    let grid = match invalidated {
        true => None,
        false => produced,
    };
    let dev = &d.dev.device;
    unsafe {
        dev.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
        .map_err(|e| format!("worker chain begin: {e}"))?;
    }

    compositor_background_two_worker_barrier::barrier::enter(&d.dev, cmd, target.image);

    // Acquire each imported window buffer before anything samples it.
    //
    // A CLIENT dmabuf comes from another driver, so it is acquired from
    // `VK_QUEUE_FAMILY_FOREIGN_EXT` — the same transfer the compositor performs,
    // gated on the same probed capability, which makes the driver interpret the
    // existing DRM-modifier/compressed contents rather than reinitialising them.
    // `oldLayout = UNDEFINED` is the content-preserving pairing for that acquire.
    //
    // A SHARED (`OPAQUE_FD`) image takes NO ownership transfer here: both ends are
    // Vulkan, so the correct family is `EXTERNAL` rather than `FOREIGN`, and a
    // correct transfer needs a matching release on the compositor's queue plus a
    // semaphore ordering the two — neither of which this path has, by design. It
    // still needs the layout transition, which is what the shader requires.
    for a in &acquires {
        let mut b = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(a.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        if a.foreign && d.dev.multiplane {
            b = b
                .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                .dst_queue_family_index(d.dev.queue_family_index);
        }
        let bs = [b];
        unsafe {
            d.dev
                .device
                .cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&bs));
        }
    }

    // Intermediates first, OUTSIDE the slot's render pass — each ends in
    // SHADER_READ_ONLY for the passes that sample it.
    if let Some(a) = after.as_mut() {
        // Stage 4: the compositor already composited background AND windows into
        // `content` and shared it. The worker decorates THAT, so its output is a
        // full world band rather than a background — which is why the compositor
        // presents it instead of drawing it underneath the windows.
        //
        // `history` is DERIVED here (previous pass's band, `worker.history`);
        // `windows` is TRANSPORTED (the engine's real layer, exported alongside
        // `content`). Both are null when no pass names them, which `input_set`
        // only ever reads for the inputs a pass actually declared.
        if let Some(h) = a.history.as_mut() {
            h.prime(d, cmd);
        }
        let hv = a.history.as_ref().map_or(vk::ImageView::null(), |h| h.view);
        graph.record_after_intermediates(&d.dev, cmd, gp, a.content, hv, a.windows, tick);
    } else {
        graph.record_intermediates(&d.dev, cmd, gp, tick);
    }
    // The warp grid, into its own tiny target and straight out to host memory —
    // same command buffer, so nothing waits.
    if let (Some(w), Some(g)) = (warpmap.as_mut(), gp.warp_map.as_ref()) {
        w.record(&d.dev, cmd, &g.push);
    }

    // The band's `output` pass draws into the slot image, where the single-pass
    // path draws its one pass. CLEAR, not LOAD: `enter` acquires the slot from
    // UNDEFINED, so there is nothing to preserve.
    compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
        &d.dev, cmd, target.view, extent, [0.0; 4], vk::AttachmentLoadOp::CLEAR,
    );
    if let Some(a) = after.as_ref() {
        let hv = a.history.as_ref().map_or(vk::ImageView::null(), |h| h.view);
        graph.draw_after_output(&d.dev, cmd, gp, a.content, hv, a.windows, tick);
    } else {
        graph.draw_output(&d.dev, cmd, gp, tick);
    }
    compositor_kernel_vulkan_pipeline_composite_base::composite::end(&d.dev, cmd);

    // Roll `history` forward ONLY once everything that samples it has been
    // recorded — it holds the PREVIOUS pass until this point, which is the whole
    // contract. Outside the slot's render pass, because it opens its own.
    if let Some(a) = after.as_ref()
        && let Some(h) = a.history.as_ref()
    {
        h.capture(d, cmd, a.content);
    }

    compositor_background_two_worker_barrier::barrier::leave(&d.dev, cmd, target.image);

    unsafe {
        dev.end_command_buffer(cmd).map_err(|e| format!("worker chain end: {e}"))?;
        let ci = vk::CommandBufferSubmitInfo::default().command_buffer(cmd);
        let submit = vk::SubmitInfo2::default().command_buffer_infos(std::slice::from_ref(&ci));
        dev.queue_submit2(d.queue.queue, &[submit], fence)
            .map_err(|e| format!("worker chain submit: {e}"))?;
    }
    Ok(Some(grid))
}
