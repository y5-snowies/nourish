use std::ops::Add;
use std::time::Duration;

#[derive(Clone)]
pub struct Solver {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
    pub value_start: f64,
    pub value_target: f64,
    pub duration: Duration,
}

pub fn solve(solver: &Solver, current: Duration) -> Option<f64> {
    if current <= Duration::from_millis(0) {
        return Some(solver.value_start);
    }

    let mut current = current;
    if current >= solver.duration.add(Duration::from_millis(30)) {
        return None;
    } else if current >= solver.duration {
        current = solver.duration;
    }

    let current_second = current.as_secs_f64();
    let stiffness = solver.stiffness;
    let damping = solver.damping;
    let mass = solver.mass;

    let zeta = damping / (2.0 * (stiffness * mass).sqrt());
    let omega0 = (stiffness / mass).sqrt();

    let progress: f64;

    if zeta < 1.0 {
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let envelope = (-zeta * omega0 * current_second).exp();
        progress = 1.0
            - envelope
                * ((omega_d * current_second).cos()
                    + (zeta * omega0 / omega_d) * (omega_d * current_second).sin());
    } else if (zeta - 1.0).abs() < 1e-9 {
        progress = 1.0 - (-omega0 * current_second).exp() * (1.0 + omega0 * current_second);
    } else {
        let omega_p = omega0 * (zeta * zeta - 1.0).sqrt();
        let r1 = -zeta * omega0 + omega_p;
        let r2 = -zeta * omega0 - omega_p;
        let c1 = r2 / (r2 - r1);
        let c2 = -r1 / (r2 - r1);
        progress = 1.0 - (c1 * (r1 * current_second).exp() + c2 * (r2 * current_second).exp());
    }

    let lerp = solver.value_start + (solver.value_target - solver.value_start) * progress;
    Some(lerp)
}
