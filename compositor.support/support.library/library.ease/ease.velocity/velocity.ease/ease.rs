pub struct EaseConfig {
    pub stiffness: f32,
    pub damping: f32,
    pub value_target: f32,
}

pub struct Ease {
    pub value_current: f32,
    pub velocity_current: f32,
}

impl Ease {
    pub fn update(&mut self, config: EaseConfig, delta_time: f32) -> Option<f32> {
        let force = (config.value_target - self.value_current) * config.stiffness;
        let damping = self.velocity_current * config.damping;
        let acc = force - damping;

        self.velocity_current += acc * delta_time;
        self.value_current += self.velocity_current * delta_time;

        let distance = (config.value_target - self.value_current).abs();
        let velocity_threshold = 0.01;
        let position_threshold = 0.001;

        if distance < position_threshold && self.velocity_current.abs() < velocity_threshold {
            self.value_current = config.value_target;
            self.velocity_current = 0.0;
            return None;
        }

        Some(self.value_current)
    }
}
