use compositor_orchestration_draw_dispatch_frame::ParallaxUniforms;
use smithay::backend::renderer::gles::Uniform;
use std::time::Instant;

#[derive(Clone)]
pub struct Motion {
    pub velocity: (f32, f32),
    pub flow_offset: (f32, f32),
    pub pan_previous: (f32, f32),
    pub time: Instant,
    pub lock_amount: f32,
}

impl Motion {
    pub fn new() -> Self {
        Self {
            velocity: (0.0, 0.0),
            flow_offset: (0.0, 0.0),
            pan_previous: (0.0, 0.0),
            time: Instant::now(),
            lock_amount: 0.0,
        }
    }

    /// Call right before draw to splice the previous pan.
    pub fn tick(&mut self, pan: (f32, f32), locked: bool) {
        let now = Instant::now();
        let dt = (now - self.time).as_secs_f32().max(1e-4);

        let raw_velocity = (
            (pan.0 - self.pan_previous.0) / dt,
            (pan.1 - self.pan_previous.1) / dt,
        );

        // Smooth + decay, scaled by dt.
        //
        // These were per-CALL constants (0.85 smoothing, 0.95 decay), which made
        // them wrong in two ways. Per FRAME: a 144Hz output damped the flow well
        // over twice as fast as a 60Hz one. Per OUTPUT: `update()` is invoked once
        // per output pass, so with two monitors both constants applied twice per
        // frame and the parallax settled at roughly double the intended rate.
        //
        // Raising each to `dt / REF_DT` keeps the tuned behaviour exactly at 60Hz
        // and makes it frame-rate independent everywhere else. It also makes a
        // repeat call within one frame a no-op (dt ~ 0 => exponent ~ 0 => factor
        // ~ 1), so the per-output-pass call is harmless by construction rather
        // than by the caller remembering not to do it.
        const REF_DT: f32 = 1.0 / 60.0;
        let frames = dt / REF_DT;
        let s = 0.85f32.powf(frames);
        self.velocity.0 = self.velocity.0 * s + raw_velocity.0 * (1.0 - s);
        self.velocity.1 = self.velocity.1 * s + raw_velocity.1 * (1.0 - s);

        // Decay so it settles when idle (avoids drift forever)
        let decay = 0.95f32.powf(frames);
        self.velocity.0 *= decay;
        self.velocity.1 *= decay;

        // INTEGRATE velocity into flow_offset
        self.flow_offset.0 += self.velocity.0 * dt * 0.0005;
        self.flow_offset.1 += self.velocity.1 * dt * 0.0005;

        self.pan_previous = pan;

        let target = if locked { 1.0 } else { 0.0 };

        let dir = target - self.lock_amount;
        if dir != 0.0 {
            const LOCK_FADE: f32 = 1.0; // seconds for a full transition
            let step = dt / LOCK_FADE;
            self.lock_amount += dir.signum() * step.min(dir.abs()); // never overshoot
        }

        self.time = now;
    }
}

/// GLES uniforms + the renderer-agnostic uniforms (same values) for one draw.
/// `params` are the shader-authored `@prop` values, packed into four `vec4`
/// uniforms `u_param0`..`u_param3` (matching the fixed params block on Vulkan).
#[allow(clippy::too_many_arguments)]
pub fn uniforms(
    time: f32,
    lock_amount: f32,
    pan: (f32, f32),
    flow_offset: (f32, f32),
    velocity: (f32, f32),
    zoom: f32,
    resolution: (f32, f32),
    params: &[f32; 16],
    srgb: bool,
) -> (Vec<Uniform<'static>>, ParallaxUniforms) {
    let gles = vec![
        Uniform::new("u_time", time),
        Uniform::new("u_lock_amount", lock_amount),
        Uniform::new("u_pan", [pan.0, pan.1]),
        Uniform::new("u_flow_offset", [flow_offset.0, flow_offset.1]),
        Uniform::new("pan_velocity", [velocity.0, velocity.1]),
        Uniform::new("u_zoom", zoom),
        Uniform::new("u_resolution", [resolution.0, resolution.1]),
        Uniform::new("u_param0", [params[0], params[1], params[2], params[3]]),
        Uniform::new("u_param1", [params[4], params[5], params[6], params[7]]),
        Uniform::new("u_param2", [params[8], params[9], params[10], params[11]]),
        Uniform::new("u_param3", [params[12], params[13], params[14], params[15]]),
    ];

    let vk = ParallaxUniforms {
        resolution: [resolution.0, resolution.1],
        zoom,
        time,
        pan: [pan.0, pan.1],
        flow_offset: [flow_offset.0, flow_offset.1],
        velocity: [velocity.0, velocity.1],
        lock_amount,
        alpha: 1.0,
        srgb: if srgb { 1.0 } else { 0.0 },
    };

    (gles, vk)
}
