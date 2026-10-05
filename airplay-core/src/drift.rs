//! Slow PI servo on total unsent PCM, including Rust queue and OS pipe.
//! Positive water error reduces output/input ratio (negative ppm correction).
use serde::Serialize;
#[derive(Serialize)]
pub struct Controller {
    pub updates: u64,
    pub target_ms: f64,
    pub filtered_ms: f64,
    pub correction_ppm: f64,
    pub min_water_ms: f64,
    pub max_water_ms: f64,
    integral_ppm: f64,
    warmup_sum: f64,
    pub saturated_updates: u64,
}
impl Controller {
    pub fn new() -> Self {
        Self {
            updates: 0,
            target_ms: 0.0,
            filtered_ms: 0.0,
            correction_ppm: 0.0,
            min_water_ms: f64::MAX,
            max_water_ms: f64::MIN,
            integral_ppm: 0.0,
            warmup_sum: 0.0,
            saturated_updates: 0,
        }
    }
    pub fn update(&mut self, water_ms: f64, dt: f64) -> f64 {
        self.updates += 1;
        self.min_water_ms = self.min_water_ms.min(water_ms);
        self.max_water_ms = self.max_water_ms.max(water_ms);
        // Preserve the actual startup depth, including fixed packet/filter phase.
        if self.updates <= 5 {
            self.warmup_sum += water_ms;
            self.target_ms = self.warmup_sum / self.updates as f64;
            self.filtered_ms = self.target_ms;
            return 0.0;
        }
        let dt = dt.clamp(0.01, 2.0);
        self.filtered_ms += (water_ms - self.filtered_ms) * (1.0 - (-dt / 3.0).exp());
        let error = self.filtered_ms - self.target_ms;
        let proposed = (self.integral_ppm - 0.2 * error * dt).clamp(-800.0, 800.0);
        let raw = -20.0 * error + proposed;
        // Anti-windup: only integrate at a limit if it moves toward the interior.
        if raw.abs() < 800.0 || raw * (proposed - self.integral_ppm) < 0.0 {
            self.integral_ppm = proposed;
        }
        let target = (-20.0 * error + self.integral_ppm).clamp(-800.0, 800.0);
        if target.abs() >= 799.9 {
            self.saturated_updates += 1;
        }
        self.correction_ppm += (target - self.correction_ppm).clamp(-20.0 * dt, 20.0 * dt);
        self.correction_ppm
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_hour_clock_mismatch_and_jitter_stay_bounded() {
        for hardware in [-500.0, -100.0, 0.0, 100.0, 500.0] {
            let mut controller = Controller::new();
            let mut water = 136.0;
            for second in 0..7200 {
                let jitter = 4.0 * (second as f64 * 1.7).sin();
                let ppm = controller.update(water + jitter, 1.0);
                water += (hardware + ppm * (1.0 + hardware / 1e6)) * 0.001;
                assert!(
                    (90.0..180.0).contains(&water),
                    "hardware={hardware} water={water}"
                );
            }
            assert!((controller.correction_ppm + hardware).abs() < 50.0);
        }
    }
    #[test]
    fn rate_step_slew_limit_and_limit_recovery() {
        let mut controller = Controller::new();
        let mut water = 136.0;
        let mut previous = 0.0;
        for second in 0..10800 {
            let hardware = if second < 3600 { 300.0 } else { -300.0 };
            let ppm = controller.update(water, 1.0);
            assert!((ppm - previous).abs() <= 20.000001);
            assert!(ppm.abs() <= 800.0);
            water += (hardware + ppm) * 0.001;
            assert!((90.0..180.0).contains(&water));
            previous = ppm;
        }
        assert!((controller.correction_ppm - 300.0).abs() < 1.0);
        for _ in 0..1000 {
            controller.update(1000.0, 1.0);
        }
        assert_eq!(controller.correction_ppm, -800.0);
        for _ in 0..1000 {
            controller.update(controller.target_ms - 10.0, 1.0);
        }
        assert!(controller.correction_ppm > 0.0);
    }
}
