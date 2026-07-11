//! Secondary DRM device assembly (multi-GPU). After the PRIMARY card has booted,
//! open every OTHER KMS-capable card, register it in the SHARED `GpuManager`, build
//! its own `DrmOutputManager`, and light its connected connectors — composited
//! LOCALLY on that card (each card drives its own monitors; no cross-device copy).
//!
//! **Additive & safe to single-device:** a single-card system finds no other KMS
//! card and this is a no-op, so the primary boot path is physically untouched. Every
//! new device/pipe carries its own `device` node, so vblank routing (`(device, crtc)`)
//! and manager selection stay correct.
//!
//! **Runtime-unverified without real multi-GPU hardware:** the per-card modeset, the
//! local render, and the per-card DRM-master handshake only exercise on a box with a
//! second KMS card. This compiles and cannot regress single-device by construction.

use std::cell::RefCell;
use std::rc::Rc;

use compositor_kernel_native_context_render_base::render::{DeviceRender, NativeRenderContext};
use compositor_orchestration_core_state_base::Loop;
use smithay::backend::drm::{DrmNode, NodeType};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::connector;

/// Open + drive every KMS-capable card that is NOT the primary (`primary_dev_id`).
/// No-op when there is only the primary card.
pub fn open_secondary_devices(
    session: &mut LibSeatSession,
    seat_name: &str,
    primary_dev_id: u64,
    event_loop: &mut EventLoop<'static, Loop>,
    loop_: &mut Loop,
    ctx_rc: &Rc<RefCell<NativeRenderContext>>,
) {
    let cards = compositor_kernel_udev_enumerate_scan_base::scan::snapshot(seat_name);
    let ten_bit = compositor_developer_environment_config_base::base::get().depth == 10;
    // The shared GpuManager (every card's node registers here; renderers per card).
    let gpu_binding = ctx_rc.borrow().gpu_binding.clone();

    for (dev_id, path) in cards {
        if dev_id == primary_dev_id {
            continue;
        }
        // Only KMS CARD nodes are scanout candidates. The udev DRM enumeration can
        // include RENDER nodes (e.g. `renderD*`) — those are NOT seat-managed, so
        // seat-opening one is a fatal ENODEV. Resolve + filter the node type BEFORE
        // opening anything.
        let Ok(node) = DrmNode::from_path(&path) else {
            continue;
        };
        if node.ty() != NodeType::Primary {
            continue; // render node — not a scanout card
        }
        // Best-effort open: a card we can't open (permissions / master held elsewhere)
        // is SKIPPED, not fatal (unlike the required primary device).
        let Some(owned) = compositor_kernel_seat_interface_open_base::open::try_open(session, &path)
        else {
            warn!("secondary card {path:?}: could not open — skipping");
            continue;
        };
        let fd = compositor_kernel_drm_device_open_base::open::wrap_fd(owned);
        // Functional KMS probe — a card with no modeset capability drives no monitors.
        let cap = compositor_kernel_drm_device_capability_base::capability::probe(&fd);
        if !cap.is_scanout_capable() {
            continue;
        }

        let gbm = compositor_kernel_drm_gbm_device_base::device::create(fd.clone());
        let (drm, notifier) = compositor_kernel_drm_device_open_base::open::open(fd.clone());

        // Skip cards with NO connected monitor (e.g. the render-only dGPU on a PRIME
        // laptop, whose panel hangs off the iGPU) — don't build a phantom device or a
        // useless vblank source for it. Scan off the raw `drm` before it moves into
        // the manager; the `connector::Info`s are owned snapshots, still valid after.
        let connectors: Vec<connector::Info> = {
            let res = compositor_kernel_drm_connector_scan_base::scan::resources(&drm);
            compositor_kernel_drm_connector_scan_base::scan::connectors(&drm, &res)
                .into_iter()
                .filter(|c| c.state() == connector::State::Connected)
                .collect()
        };
        if connectors.is_empty() {
            info!("secondary card {path:?}: no connected monitor — skipping");
            continue;
        }

        // Register the card in the shared GpuManager (with ITS OWN gbm) so
        // `single_renderer(node)` composites on this card, and read its render formats.
        let render_formats = {
            let mut binding = gpu_binding.borrow_mut();
            compositor_kernel_gles_multigpu_factory_base::factory::add_node(
                &mut binding.gpus,
                node,
                gbm.clone(),
            );
            let mut r = compositor_kernel_gles_multigpu_factory_base::factory::single_renderer(
                &mut binding.gpus,
                &node,
            );
            r.as_mut().egl_context().dmabuf_render_formats().clone()
        };

        let allocator = compositor_kernel_drm_gbm_alloc_base::alloc::allocator(gbm.clone());
        let exporter = compositor_kernel_drm_gbm_alloc_base::alloc::exporter(gbm.clone(), node);
        let mgr = compositor_kernel_scanout_surface_output_base::output::manager(
            drm,
            allocator,
            exporter,
            Some(gbm.clone()),
            render_formats,
            ten_bit,
        );
        let mgr_rc = Rc::new(RefCell::new(mgr));

        // Publish the device + its own vblank source BEFORE lighting outputs, so
        // vblank routing finds it and `add_output_on` sees a live manager.
        ctx_rc.borrow_mut().devices.push(DeviceRender {
            node,
            drm_fd: fd.clone(),
            drm_output_manager: mgr_rc.clone(),
            vblank_token: None,
        });
        let token = compositor_kernel_native_wire_frame_base::frame::register(
            event_loop,
            loop_,
            notifier,
            node,
            ctx_rc.clone(),
        );
        if let Some(d) = ctx_rc.borrow_mut().device_mut(&node) {
            d.vblank_token = Some(token);
        }

        // Light each connected connector on this card, composited locally on `node`.
        let mut lit = 0usize;
        for conn in &connectors {
            let mut ctx = ctx_rc.borrow_mut();
            match compositor_kernel_native_context_display_reconcile::reconcile::add_output_on(
                loop_,
                &mut ctx,
                &mgr_rc,
                node,
                Some(node),
                conn,
                None,
            ) {
                Ok(()) => lit += 1,
                Err(e) => {
                    warn!("secondary card {path:?}: connector {:?} failed: {e}", conn.handle())
                }
            }
        }
        info!(
            "secondary card {path:?} (node {node:?}): {lit}/{} connector(s) lit",
            connectors.len()
        );
    }
}
