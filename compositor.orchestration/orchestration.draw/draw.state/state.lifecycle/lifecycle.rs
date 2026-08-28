use smithay::backend::input::{InputBackend, InputEvent};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};
use compositor_introspection_sampler_window_base::sampler::{SampleBatch, SampleResult};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_display_output::backend::Backend;
use compositor_y5_graphic_display_output::output;

pub fn initialize(
    _loop: &mut Loop,
    output: &Output,
    display_handle: &DisplayHandle,
    backend: &mut dyn Backend,
) -> OutputDamageTracker {
    // Create the backend. This creates a winit/udev instance
    // let (backend) = backend_loader.load();

    // Registers the output in smithay space. ( monitor _
    output::register(_loop, output);

    // Creates the damage tracker
    let output_damage_tracker = OutputDamageTracker::from_output(&output);

    // Allows smithay to render DMABUF imports from clients, and registers the EGL
    // import set with the format layer.
    //
    // ADVERTISING them is a separate step now (`output::advertise_dmabuf`, driven
    // by the loader). Building the feedback here meant answering the format layer
    // in the middle of the registration phase, while the wgpu adapters were still
    // probing — so the advertisement was computed from an unfinished registrar.
    output::bind_display(_loop, backend);

    // Damage tracker created for the output(monitor)
    output_damage_tracker
}

pub fn input<I: InputBackend>(_loop: &mut Loop, input_event: &InputEvent<I>) {
    // return self._loop..process_input_event(input_event);
    compositor_orchestration_seat_delegate_base::delegate::process_input_event(_loop, input_event)
}

pub fn stop(_loop: &mut Loop) {
    _loop.inner.loader.loop_signal.stop();
}

pub fn resize(
    output: Output,
    size: Size<i32, Physical>,
    scale_factor: Option<smithay::output::Scale>,
) {
    output.change_current_state(
        Some(Mode {
            size,
            refresh: 60_000,
        }),
        None,
        scale_factor,
        None,
    );
}

pub fn sampler_result(_loop: &mut Loop, result: SampleBatch) {
    compositor_y5_placeholder_interface_base::interface::on_window_sample(_loop, &result);
}
