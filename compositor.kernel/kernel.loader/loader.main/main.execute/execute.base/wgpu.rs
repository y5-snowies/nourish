use std::sync::{Arc, mpsc};
use compositor_model_debug_instance_record::{error, info};

/// Spawn the wgpu init thread; returns (bevy ctx receiver, iced ctx receiver).
/// The bevy receiver is handed to ThreeSystem at construction (system-private
/// machinery); the iced one still lands in surface state until that system
/// extracts (phase 4).
pub fn initialize_wgpu_context(
    formats: compositor_kernel_graphic_format_registrar_base::registrar::Registrar,
) -> (
    mpsc::Receiver<compositor_support_bevy_core_runtime_base::WgpuVulkanContext>,
    mpsc::Receiver<Arc<compositor_monitor_runtime_surface_base::WgpuVulkanContext>>,
) {
    // Due to wayland requirements of vulkan
    info!("Dispatching WGPU Context creation to a background thread");
    let (tx, rx) = mpsc::channel();
    let (tx_iced_wgpu, rx_iced_wgpu) = mpsc::channel();

    std::thread::spawn(move || {
        // The handle is MOVED into the thread — this is the whole point of the
        // registrar being a value rather than a global: the adapters are probed
        // off-thread and register from there.
        let iced_formats = formats.clone();
        let result = compositor_monitor_runtime_surface_base::create_wgpu_vulkan_context(&iced_formats);
        if result.is_err() {
            error!(
                "(iced) wgpu init thread for iced: failed: {:?}",
                result.err()
            );
            return;
        }
        info!("(iced) wgpu init thread for iced: success");

        let result = result.unwrap().into_arc();
        // If receiver dropped (main exited), send fails silently — that's fine.
        let _ = tx_iced_wgpu.send(result);

        let result = compositor_support_bevy_core_runtime_base::create_wgpu_vulkan_context(&formats);
        match &result {
            Ok(_) => info!("wgpu init thread: success"),
            Err(e) => {
                error!("wgpu init thread: failed: {:?}", e);
                return;
            }
        }
        // If receiver dropped (main exited), send fails silently — that's fine.
        let result = result.unwrap();
        let _ = tx.send(result);
    });

    (rx, rx_iced_wgpu)
}
