// No logging here on purpose: this runs per pointer motion for the whole
// duration of a drag, so a record per event would be pure noise. The protocol
// side (`dispatch.wire/wire.drag`) logs the state changes instead.

pub mod drag_state;
