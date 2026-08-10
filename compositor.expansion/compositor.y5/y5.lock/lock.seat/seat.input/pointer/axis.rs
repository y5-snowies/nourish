use smithay::{
    backend::input::{Axis, AxisSource, Event, InputBackend, PointerAxisEvent},
    input::pointer::AxisFrame,
};
use compositor_orchestration_core_state_base::Loop;


pub fn axis<I: InputBackend>(event: &<I as InputBackend>::PointerAxisEvent, _loop: &mut Loop) {
    // Needs to select for the specific layer only.
    //
    //  let pointer = _loop.state.seat.seat.get_pointer().unwrap();
    // let source = event.source();

    //  let horizontal_amount = event
    //      .amount(Axis::Horizontal)
    //      .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
    //  let vertical_amount = event
    //      .amount(Axis::Vertical)
    //      .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
    //  let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
    //  let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

    //  let mut frame = AxisFrame::new(event.time_msec()).source(source);
    //  if horizontal_amount != 0.0 {
    //      frame = frame.value(Axis::Horizontal, horizontal_amount);
    //      if let Some(discrete) = horizontal_amount_discrete {
    //          frame = frame.v120(Axis::Horizontal, discrete as i32);
    //      }
    //  }
    //  if vertical_amount != 0.0 {
    //      frame = frame.value(Axis::Vertical, vertical_amount);
    //      if let Some(discrete) = vertical_amount_discrete {
    //          frame = frame.v120(Axis::Vertical, discrete as i32);
    //      }
    //  }

    //  if source == AxisSource::Finger {
    //      if event.amount(Axis::Horizontal) == Some(0.0) {
    //          frame = frame.stop(Axis::Horizontal);
    //      }
    //      if event.amount(Axis::Vertical) == Some(0.0) {
    //          frame = frame.stop(Axis::Vertical);
    //      }
    //  }

    //  if let Some(registry) = compositor_y5_lock_system_base::base::registry(&mut _loop.inner.worlds) {
    //      // ── Iced axis ─────────────────────────────────────────────────
    //      let iced_target = registry.pointer_target();
    //      let discrete_x = horizontal_amount_discrete
    //          .map(|v| (v / 120.0) as i32)
    //          .unwrap_or(0);
    //      let discrete_y = vertical_amount_discrete
    //          .map(|v| (v / 120.0) as i32)
    //          .unwrap_or(0);
    //      registry.dispatch_axis(
    //          iced_target,
    //          discrete_x,
    //          discrete_y,
    //          horizontal_amount,
    //          vertical_amount,
    //      );
    //  }
}
