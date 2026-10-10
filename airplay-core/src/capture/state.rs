//! 区分已观察的采集进度与 sink 成功接收的交付计数。
//! Separate observed capture progress from counters committed after successful sink delivery.
use super::{CaptureProgress, metrics::PacketMetrics};
use std::sync::Mutex;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR,
};

#[derive(Clone, Copy)]
pub(super) struct PacketInfo {
    pub index: u64,
    pub device_position: u64,
    pub qpc: u64,
    pub available: u32,
    pub flags: u32,
}
impl PacketInfo {
    pub fn discontinuity(self) -> bool {
        self.flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0
    }
    pub fn timestamp_error(self) -> bool {
        self.flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0
    }
}
#[derive(Default)]
pub(super) struct Counters {
    pub frames: u64,
    pub packets: u64,
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    pub signal_frames: u64,
    pub channel_peaks: [f32; 2],
    pub silent_packets: u64,
    pub silent_frames: u64,
}
impl Counters {
    pub fn observe(&mut self, packet: PacketInfo, metrics: &PacketMetrics) {
        // 首包 discontinuity 是 WASAPI 启动标志，报告仍排除它；实时进度保留原始标志。
        // The first discontinuity is a WASAPI startup flag; reports exclude it, progress retains it.
        self.discontinuities += u64::from(packet.discontinuity() && self.packets > 0);
        self.timestamp_errors += u64::from(packet.timestamp_error());
        self.signal_frames += metrics.signal_frames;
        for (total, peak) in self.channel_peaks.iter_mut().zip(metrics.peaks) {
            *total = total.max(peak);
        }
    }
    pub fn delivered_packet(&mut self, frames: usize) {
        self.frames += frames as u64;
        self.packets += 1;
    }
    pub fn delivered_silence(&mut self, frames: u64) {
        self.frames += frames;
        self.silent_frames += frames;
    }
}

pub(super) struct Progress<'a>(pub Option<&'a Mutex<CaptureProgress>>);
impl Progress<'_> {
    fn update(&self, apply: impl FnOnce(&mut CaptureProgress)) {
        if let Some(progress) = self.0 {
            apply(&mut progress.lock().unwrap());
        }
    }
    pub fn fallback(&self) {
        self.update(|p| p.event_timeout_packets += 1);
    }
    pub fn silence(&self, count: u64) {
        self.update(|p| p.synthesized_silent_frames += count);
    }
    pub fn gap(&self, count: u64) {
        self.update(|p| {
            p.repaired_gaps += 1;
            p.repaired_gap_frames += count;
        });
    }
    pub fn packet(
        &self,
        packet: PacketInfo,
        skipped: usize,
        metrics: &PacketMetrics,
    ) -> Option<CaptureProgress> {
        self.0.map(|progress| {
            let mut p = progress.lock().unwrap();
            p.packets += 1;
            p.skipped_overlap_frames += skipped as u64;
            p.device_position = packet.device_position;
            p.packet_qpc_100ns = packet.qpc;
            p.packet_frames = packet.available;
            p.packet_flags = packet.flags;
            p.packet_peaks = metrics.peaks;
            p.signal_frames += metrics.signal_frames;
            p.discontinuities += u64::from(packet.discontinuity());
            p.timestamp_errors += u64::from(packet.timestamp_error());
            *p
        })
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/state.rs"]
mod tests;
