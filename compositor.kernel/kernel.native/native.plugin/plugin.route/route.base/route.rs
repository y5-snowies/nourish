//! Reacts to typed device events: delegate (classify) -> select -> bookkeep,
//! with the crash-first policy on events the compositor cannot survive.
//! Detection lives in backend.udev; this is the host-side reaction.
//!
//! - Added(new, preference-allowed)  -> register (bookkeeping; single-GPU
//!   policy drives only the primary)
//! - Added(known)                    -> ignore (re-announce)
//! - Changed(primary)                -> connector rescan + diff; ANY change on the
//!   driven device returns `true` so the caller reconciles the active output
//!   (fail over to another connected monitor, or go dark + wait — see
//!   `display.reconcile::reconcile`). No longer panics on active-connector loss.
//! - Removed(primary)                -> panic (the device under the running
//!   pipe is gone; not self-recovering)
//! - Removed(secondary)              -> bookkeeping cleanup
//!
//! Returns `true` when the caller should reconcile the active output.

use compositor_kernel_drm_connector_diff_base::diff;
use compositor_kernel_gpu_registry_node_base::node::NodeRegistry;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_native_context_topology_base::topology::Topology;
use compositor_kernel_udev_loop_event_base::event::DeviceEvent;
use compositor_kernel_graphic_preference_gpu_rank::rank::GpuRank;
use std::cell::RefCell;
use std::rc::Rc;

pub fn route(
    event: DeviceEvent,
    registry: &mut NodeRegistry,
    topology: &mut Topology,
    rank: &GpuRank,
    ctx_rc: &Rc<RefCell<NativeRenderContext>>,
) -> bool {
    use compositor_kernel_native_device_delegate_base::delegate::{classify, DeviceClass};
    use compositor_kernel_native_device_select_base::select::{decide, Decision};

    match event {
        DeviceEvent::Added { device_id, path } => {
            match classify(device_id, Some(&path), registry) {
                DeviceClass::KnownGpu => {
                    trace!("device re-announced; ignoring: {device_id:?}");
                }
                DeviceClass::NewGpu => match decide(&path, rank) {
                    Decision::Ignore => {
                        info!("DRM device preference-excluded; not bookkeeping: {path:?}");
                    }
                    Decision::Register => {
                        let node = compositor_kernel_drm_device_node_base::node::render_node(&path)
                            .unwrap_or_else(|| {
                                abort!("udev announced a DRM device with no resolvable node: {path:?}")
                            });
                        registry.add(device_id, node);
                        topology.register_device(device_id, node);
                        info!(
                            "DRM device registered (single-GPU policy: only the \
                             primary is driven): {path:?}"
                        );
                    }
                },
                DeviceClass::Unknown => {}
            }
            false
        }
        DeviceEvent::Changed { device_id } => {
            // Match the udev device against the primary GPU by BOTH its render and
            // card dev_id — udev reports the CARD node's dev_t for connector hotplug,
            // but the registry stores the RENDER node. Without this every hotplug was
            // wrongly dismissed as "non-driven device".
            let is_primary = registry.is_primary_dev(device_id);
            info!("udev Changed: device {device_id:?} (primary={is_primary})");
            if !is_primary {
                trace!("change on non-driven device; bookkeeping only: {device_id:?}");
                return false;
            }

            // Connector rescan on the driven device (best-effort log of the diff).
            let ctx = ctx_rc.borrow();
            let manager = ctx.drm_output_manager.borrow();
            let drm = manager.device();
            let res = compositor_kernel_drm_connector_scan_base::scan::resources(drm);
            let infos = compositor_kernel_drm_connector_scan_base::scan::connectors(drm, &res);
            let new_snapshot = diff::ConnectorSnapshot::take(&infos);
            drop(manager);
            drop(ctx);

            let old = topology.snapshot(device_id).cloned().unwrap_or_default();
            let changes = diff::diff(&old, &new_snapshot);
            topology.set_snapshot(device_id, new_snapshot);
            info!(
                "primary device changed: +{} -{} connector(s) → reconciling output",
                changes.connected.len(),
                changes.disconnected.len()
            );
            // Always reconcile on a primary change — `reconcile` re-scans the live
            // connectors and decides (fail over / recover / refresh) authoritatively,
            // so it does NOT depend on the (separately-keyed) topology diff above.
            true
        }
        DeviceEvent::Removed { device_id } => {
            let was_primary = registry.is_primary_dev(device_id);
            if was_primary {
                abort!("primary DRM device removed from under the running pipe");
            }
            registry.remove(device_id);
            topology.remove_device(device_id);
            info!("secondary DRM device removed; bookkeeping cleaned: {device_id:?}");
            false
        }
    }
}
