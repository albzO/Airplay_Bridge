//! 缓慢的 PI 时钟漂移控制，输入是整条管线的 PCM 水位（ms），包括队列和管道。
//! 前 5 次更新以实际启动深度建立基准，避免把滤波器/分包相位误认为时钟漂移。
//! 水位偏高意味着输出太快，返回负 ppm 减少输出/输入比例；偏低则反向调整。
//! 校正幅度限制 ±800 ppm，变化速度限制每秒 20 ppm，避免把包抖动变成突变。
//!
//! Slow PI clock-drift control uses total PCM water level (ms), including queues and pipes.
//! The first five updates establish the actual startup baseline, including filter/packet phase.
//! High water means output is too fast: negative ppm reduces the output/input ratio; low water
//! does the reverse. Limit correction to ±800 ppm and slew to 20 ppm/s to avoid chasing jitter.
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
    /// dt 是本次更新间隔（秒），返回相对采样率修正量（ppm），传给 Converter。
    /// dt is the update interval in seconds; return a sample-rate correction in ppm for Converter.
    pub fn update(&mut self, water_ms: f64, dt: f64) -> f64 {
        self.updates += 1;
        self.min_water_ms = self.min_water_ms.min(water_ms);
        self.max_water_ms = self.max_water_ms.max(water_ms);
        // 启动预缓冲和滤波器延迟已体现在真实水位中，保留它而非强行追固定数值。
        // Preserve measured startup depth, which already includes prebuffer and filter delay.
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
        // 抗积分饱和：达到限幅后只允许把修正量拉回范围内的积分继续累积。
        // Anti-windup: at saturation, integrate only when doing so moves correction back inward.
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
#[path = "../../test/core/unit/drift.rs"]
mod tests;
