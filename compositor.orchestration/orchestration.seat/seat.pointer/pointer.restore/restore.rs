//! Hide-on-touch / restore-on-pointer for the single shared seat cursor. Seat-pointer
//! policy, so it lives in the pointer chain rather than on the state root — the
//! Orchestrator keeps only the `saved_cursor` value (see `pointer.snapshot`).

use compositor_orchestration_core_state_base::{Loop, state::output_key};
use compositor_orchestration_seat_pointer_snapshot::snapshot::CursorSnapshot;
use smithay::backend::input::{InputBackend, InputEvent};
use smithay::input::pointer::MotionEvent;
use smithay::utils::SERIAL_COUNTER;

/// The single shared cursor "disappears" while a touch sequence owns the active
/// output, and is restored the instant a real pointer / pen event arrives.
///
/// The hidden state is deliberately STICKY across the end of a touch sequence: it is
/// cleared by pointer use, not by lifting the last finger, so a touch-only session
/// never flashes a cursor between taps. On a device with no pointer at all the cursor
/// therefore stays hidden for good, which is the intent.
pub trait TouchCursor {
    /// Entering touch: snapshot the cursor once (no-op if already held).
    fn touch_enter(&mut self);
    /// If a pointer/tablet/gesture event arrived while touch held the cursor, restore
    /// the snapshot (position + active output) and show the cursor again. Keyboard /
    /// touch / device / switch events are not pointer use.
    fn touch_restore_if_pointer<I: InputBackend>(&mut self, event: &InputEvent<I>);
}

impl TouchCursor for Loop {
    fn touch_enter(&mut self) {
        if self.inner.saved_cursor.is_some() {
            return;
        }
        let Some(location) = self.state.seat.seat.get_pointer().map(|p| p.current_location()) else {
            return;
        };
        self.inner.saved_cursor = Some(CursorSnapshot {
            motion: self.inner.pointer().motion,
            output: self.inner.cursor_output.clone(),
            location,
        });
    }

    fn touch_restore_if_pointer<I: InputBackend>(&mut self, event: &InputEvent<I>) {
        use InputEvent::*;
        let pointer_used = !matches!(
            event,
            Keyboard { .. }
                | TouchDown { .. }
                | TouchMotion { .. }
                | TouchUp { .. }
                | TouchCancel { .. }
                | TouchFrame { .. }
                | SwitchToggle { .. }
                | DeviceAdded { .. }
                | DeviceRemoved { .. }
                | Special(_)
        );
        if !pointer_used {
            return;
        }
        let Some(snap) = self.inner.saved_cursor.take() else {
            return;
        };
        self.inner.pointer_mut().motion = snap.motion;

        // The snapshotted output may have been unplugged or disabled while the touch
        // sequence was running. Re-pinning to it would resurrect a view tree for a
        // monitor that no longer exists (`set_current` -> `ensure` creates on miss)
        // and strand the cursor on it, so fall back to leaving the current output
        // alone when the key is stale.
        let live = snap
            .output
            .as_ref()
            .is_some_and(|key| self.inner.space_state().state.outputs().any(|o| output_key(o) == *key));
        if live {
            self.inner.cursor_output = snap.output.clone();
            if let Some(key) = &snap.output {
                self.inner.output_views_mut().set_current(key);
            }
        }

        // Re-place the seat cursor where it was, so a pointer button (no motion) acts
        // at the right spot and the reappearing cursor doesn't flash at the touch
        // location; a following relative motion re-derives it anyway.
        if let Some(pointer) = self.state.seat.seat.get_pointer() {
            let serial = SERIAL_COUNTER.next_serial();
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);
            pointer.motion(
                &mut self.state,
                None,
                &MotionEvent { location: snap.location, serial, time },
            );
            pointer.frame(&mut self.state);
        }
    }
}
