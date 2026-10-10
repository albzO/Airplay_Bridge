//! 采集包统计与诊断时序；只读样本并使用调用者提供的时钟，不访问 WASAPI。
//! Packet statistics and diagnostic timing; read-only samples and caller-provided clocks, no WASAPI.

use std::time::Instant;

pub(super) struct PacketMetrics {
    pub peaks: [f32; 2],
    pub signal_frames: u64,
    pub fingerprint: Option<u64>,
}

impl PacketMetrics {
    pub fn measure(samples: &[f32], fingerprint: bool) -> Self {
        let mut result = Self {
            peaks: [0.0; 2],
            signal_frames: 0,
            fingerprint: fingerprint.then(|| {
                samples.iter().fold(0xcbf29ce484222325u64, |hash, sample| {
                    (hash ^ sample.to_bits() as u64).wrapping_mul(0x100000001b3)
                })
            }),
        };
        for frame in samples.chunks_exact(2) {
            let mut signal = false;
            for (channel, sample) in frame.iter().enumerate() {
                if sample.is_finite() {
                    result.peaks[channel] = result.peaks[channel].max(sample.abs());
                    signal |= sample.abs() > 1e-6;
                }
            }
            result.signal_frames += u64::from(signal);
        }
        result
    }
}

#[derive(Default)]
pub(super) struct DiagnosticClock {
    previous: Option<(Instant, u64, u64)>,
}

pub(super) struct PacketDelta {
    pub read_interval_ms: Option<f64>,
    pub device_delta_frames: Option<i128>,
    pub timestamp_delta_ms: Option<f64>,
}

impl DiagnosticClock {
    // Observe only emitted diagnostics, preserving intervals across disabled logging periods.
    // Signed differences describe regressions without underflow; timeline validation stays separate.
    pub fn observe(
        &mut self,
        received: Instant,
        device_position: u64,
        qpc_100ns: u64,
    ) -> PacketDelta {
        let delta = PacketDelta {
            read_interval_ms: self
                .previous
                .map(|p| received.duration_since(p.0).as_secs_f64() * 1000.0),
            device_delta_frames: self.previous.map(|p| device_position as i128 - p.1 as i128),
            timestamp_delta_ms: self
                .previous
                .map(|p| (qpc_100ns as i128 - p.2 as i128) as f64 / 10000.0),
        };
        self.previous = Some((received, device_position, qpc_100ns));
        delta
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/metrics.rs"]
mod tests;
