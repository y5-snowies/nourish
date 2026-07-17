/// What a receiver does with an input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFlow {
    /// The event was handled; traversal stops.
    Consume,
    /// Not interested; the bus continues to the next receiver.
    Pass,
}

/// Kernel-level input event. Platform-free on purpose: the orchestration layer
/// translates smithay/libinput/winit events into this shape before the bus
/// traversal, so systems never see backend types.
#[derive(Clone, Debug)]
pub enum InputEvent {
    Keyboard {
        /// Raw keycode (evdev).
        code: u32,
        pressed: bool,
        /// Active modifier bits (shift/ctrl/alt/logo packed by the driver).
        modifiers: u32,
    },
    PointerMotion {
        /// Position in the world's storage space (the post-constraint normalized
        /// world point — what `pointer.motion` is given as its location).
        x: f64,
        y: f64,
        /// Physical screen-space cursor position (the rim's `raw_pos` /
        /// `position_screen` accumulator). Carried separately from the world
        /// point because the canvas-pan delta (`screen - position_previous`)
        /// must run in physical space, while `pointer.motion`/transforms use the
        /// world point above.
        screen_x: f64,
        screen_y: f64,
        delta_x: f64,
        delta_y: f64,
    },
    PointerButton {
        /// Raw button code (evdev, e.g. BTN_LEFT).
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
    },
    PointerAxis {
        horizontal: f64,
        vertical: f64,
        x: f64,
        y: f64,
        /// Source is a touchpad finger (or continuous device), as opposed to a
        /// discrete scroll wheel. Lets the canvas treat two-finger touchpad
        /// scroll (pan) differently from mouse-wheel scroll (zoom).
        finger: bool,
        /// A finger pan that should feed momentum (fling/coast) — true for a real
        /// touchpad glide and a 1-finger touch glide; false for a 2-finger touch
        /// pan, which is a strict 1:1 move with no coast.
        momentum: bool,
        /// The pan originates from a touchscreen (not the trackpad). Touch deltas
        /// are true 1:1 pixel motion, so the release velocity already equals the
        /// finger velocity — the camera skips the trackpad's fling boost for it,
        /// so the coast never runs faster than the drag.
        from_touch: bool,
    },
    /// Touchpad pinch gesture, translated from the libinput pinch lifecycle.
    /// Carries the cursor location (the zoom anchor) and, on `Update`, the
    /// incremental scale factor relative to the previous update (1.0 = no
    /// change). `Begin`/`End` carry `scale = 1.0` and exist so a receiver can
    /// gate the gesture (decide canvas-zoom vs. window-forward) at begin and
    /// release any latched state at end.
    PointerPinch {
        phase: PinchPhase,
        scale: f64,
        x: f64,
        y: f64,
    },
}

/// Lifecycle phase of a [`InputEvent::PointerPinch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinchPhase {
    Begin,
    Update,
    End,
}
