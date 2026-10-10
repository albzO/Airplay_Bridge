//! 纯采集时间线：缺口预算、空闲静音和恢复播放时的重叠修剪；不访问 WASAPI。
//! Pure capture timeline: gap budgets, idle silence and resumed-packet overlap; no WASAPI calls.
use super::Result;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(super) struct PacketPlan {
    pub skip: usize,
    pub used: usize,
    pub prefix_silence: u64,
    pub repaired_gap: u64,
}

pub(super) struct Timeline {
    rate: u32,
    target: u64,
    loopback: bool,
    origin_100ns: u128,
    silence: bool,
    previous: Option<(u64, u32, u64)>,
    repairs: VecDeque<(Instant, u64)>,
}
impl Timeline {
    pub fn new(rate: u32, target: u64, loopback: bool, origin_100ns: u128) -> Self {
        Self {
            rate,
            target,
            loopback,
            origin_100ns,
            silence: false,
            previous: None,
            repairs: VecDeque::new(),
        }
    }
    pub fn previous_qpc(&self) -> Option<u64> {
        self.previous.map(|(_, _, qpc)| qpc)
    }
    pub fn idle_target(&self, elapsed: Duration) -> u64 {
        (((elapsed.as_secs_f64() - 0.04).max(0.0) * self.rate as f64) as u64).min(self.target)
    }
    pub fn idle_frames(&self, frames: u64, elapsed: Duration, idle: Duration) -> u64 {
        if !self.loopback || idle < Duration::from_millis(40) {
            return 0;
        }
        self.idle_target(elapsed)
            .saturating_sub(frames)
            .min((self.rate / 100).max(1) as u64)
    }
    pub fn mark_silence(&mut self) {
        self.silence = true;
    }
    /// frames 为已交付帧数；QPC 单位为 100 ns，elapsed 与 now 由调用者提供，便于确定性测试。
    /// frames counts delivered frames; QPC uses 100 ns. Callers supply elapsed/now for deterministic tests.
    pub fn plan(
        &mut self,
        frames: u64,
        position: u64,
        available: u32,
        qpc: u64,
        timestamp_bad: bool,
        elapsed: Duration,
        now: Instant,
    ) -> Result<PacketPlan> {
        validate_packet_timestamp(self.previous_qpc(), qpc, timestamp_bad)?;
        let remaining = self.target.saturating_sub(frames);
        let mut prefix_silence = 0;
        let mut repaired_gap = 0;
        let mut skip = 0;
        if !self.loopback || !self.silence {
            if let Some(previous) = self.previous {
                repaired_gap = recoverable_gap(previous, position, qpc, self.rate, timestamp_bad)?;
                if repaired_gap > 0 {
                    while self
                        .repairs
                        .front()
                        .is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(60))
                    {
                        self.repairs.pop_front();
                    }
                    if self.repairs.len() >= 5
                        || self.repairs.iter().map(|(_, f)| *f).sum::<u64>() + repaired_gap
                            > self.rate as u64 / 2
                    {
                        return Err(
                            "采集缺口频繁发生，60 秒内超过 5 次或累计 500 ms，已停止".into()
                        );
                    }
                    self.repairs.push_back((now, repaired_gap));
                    prefix_silence = repaired_gap.min(remaining);
                }
            }
        }
        self.previous = Some((position, available, qpc));
        if self.loopback && self.silence {
            let packet_start = ((qpc as u128).saturating_sub(self.origin_100ns) * self.rate as u128
                / 10_000_000) as u64;
            if packet_start > ((elapsed.as_secs_f64() + 0.2) * self.rate as f64) as u64 {
                return Err("播放设备 loopback 时间戳超出采集时钟".into());
            }
            if packet_start > frames {
                prefix_silence = (packet_start - frames).min(remaining);
            }
            skip = (frames + prefix_silence)
                .saturating_sub(packet_start)
                .min(available as u64) as usize;
        }
        let used = (remaining - prefix_silence).min(available as u64 - skip as u64) as usize;
        if used > 0 {
            self.silence = false;
        }
        Ok(PacketPlan {
            skip,
            used,
            prefix_silence,
            repaired_gap,
        })
    }
}

// 设备缺帧只有在包时钟一致时才能恢复。
// Missing device frames are recoverable only when the packet clock agrees.
pub(super) fn recoverable_gap(
    previous: (u64, u32, u64),
    position: u64,
    qpc: u64,
    rate: u32,
    timestamp_bad: bool,
) -> Result<u64> {
    let expected = previous.0.saturating_add(previous.1 as u64);
    if position < expected {
        return Err("采集设备位置倒退或重复，无法可信对齐音频".into());
    }
    let gap = position - expected;
    if gap == 0 {
        return Ok(0);
    }
    if timestamp_bad || qpc < previous.2 {
        return Err("采集缺口的时间戳无效，无法恢复".into());
    }
    if gap > rate as u64 / 4 {
        return Err("采集缺口超过 250 ms，已停止".into());
    }
    let device_ms = (position - previous.0) as f64 * 1000.0 / rate as f64;
    let clock_ms = (qpc - previous.2) as f64 / 10000.0;
    if (device_ms - clock_ms).abs() > 5.0 {
        return Err("采集设备位置与时间戳不一致，无法恢复缺口".into());
    }
    Ok(gap)
}
pub(super) fn validate_packet_timestamp(
    previous: Option<u64>,
    current: u64,
    timestamp_bad: bool,
) -> Result<()> {
    if timestamp_bad {
        return Err("WASAPI 采集包时间戳无效，已停止以避免错误对齐".into());
    }
    if let Some(previous) = previous.filter(|previous| current <= *previous) {
        return Err(format!("WASAPI 采集包时间戳倒退或重复，已停止；previous_qpc_100ns={previous} current_qpc_100ns={current}；请查看采集诊断").into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/timeline.rs"]
mod tests;
