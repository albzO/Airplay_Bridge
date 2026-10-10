//! 采集诊断 JSON 和时间差；回调是否启用不改变音频路径。
//! Capture diagnostic JSON and deltas; enabling callbacks does not change the audio path.
use super::{
    CaptureProgress, Format, Input, Result, clock,
    metrics::{DiagnosticClock, PacketMetrics},
    state::{Counters, PacketInfo},
};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use windows::Win32::Media::Audio::IAudioClient;

pub(super) struct Trace<'a, 'b> {
    callback: Option<&'a mut dyn FnMut(Value)>,
    enabled: Option<&'b AtomicBool>,
    clock: DiagnosticClock,
}
pub(super) struct PacketTrace {
    pub packet: PacketInfo,
    pub received: Instant,
    pub elapsed_ms: f64,
    pub used: usize,
    pub skipped: usize,
    pub prefix_silence: u64,
    pub frames_before: u64,
    pub mapping: [usize; 2],
    pub raw: Value,
}
impl<'a, 'b> Trace<'a, 'b> {
    pub fn new(
        callback: Option<&'a mut dyn FnMut(Value)>,
        enabled: Option<&'b AtomicBool>,
    ) -> Self {
        Self {
            callback,
            enabled,
            clock: DiagnosticClock::default(),
        }
    }
    pub fn active(&self) -> bool {
        self.callback.is_some() && self.enabled.is_none_or(|flag| flag.load(Ordering::Relaxed))
    }
    fn emit(&mut self, entry: Value) {
        if let Some(log) = self.callback.as_mut() {
            log(entry);
        }
    }
    pub fn start(
        &mut self,
        input: &Input,
        format: &Format,
        client: &IAudioClient,
        loopback: bool,
    ) -> Result<()> {
        if self.callback.is_some() {
            let frequency = clock::frequency()?;
            self.emit(json!({"kind":"capture_start","input":input,"format":format,"qpc_frequency":frequency,
                "capture_wait":if loopback {"wasapi_event"}else{"polling"},
                "endpoint_buffer_frames":super::health::buffer_frames(client)}));
        }
        Ok(())
    }
    pub fn fallback(&mut self, elapsed_ms: f64, available: u32) {
        if self.callback.is_some() {
            self.emit(json!({"kind":"capture_event_fallback","capture_elapsed_ms":elapsed_ms,"available_frames":available}));
        }
    }
    pub fn silence(
        &mut self,
        elapsed_ms: f64,
        frames_before: u64,
        frames: u64,
        idle_ms: f64,
        delivered: &Result<()>,
    ) {
        if self.callback.is_some() {
            self.emit(json!({"kind":"synthetic_silence","capture_elapsed_ms":elapsed_ms,"frames_before":frames_before,
                "frames":frames,"idle_ms":idle_ms,"error":delivered.as_ref().err().map(ToString::to_string)}));
        }
    }
    pub fn flags(&mut self, packet: PacketInfo, elapsed_ms: f64) {
        if self.callback.is_some() && (packet.discontinuity() || packet.timestamp_error()) {
            self.emit(json!({"kind":"packet_flags","packet_index":packet.index,"capture_elapsed_ms":elapsed_ms,
                "device_position":packet.device_position,"packet_qpc_100ns":packet.qpc,"available_frames":packet.available,"flags":packet.flags}));
        }
    }
    pub fn fault(
        &mut self,
        packet: PacketInfo,
        previous_qpc: Option<u64>,
        error: &dyn std::fmt::Display,
    ) {
        if self.callback.is_some() {
            self.emit(json!({"kind":"timeline_fault","device_position":packet.device_position,
                "packet_qpc_100ns":packet.qpc,"previous_packet_qpc_100ns":previous_qpc,"flags":packet.flags,"error":error.to_string()}));
        }
    }
    pub fn gap(&mut self, packet: PacketInfo, frames: u64, rate: u32) {
        if self.callback.is_some() {
            self.emit(json!({"kind":"gap_repaired","gap_frames":frames,"gap_ms":frames as f64*1000.0/rate as f64,
                "device_position":packet.device_position,"packet_qpc_100ns":packet.qpc}));
        }
    }
    pub fn packet(&mut self, data: PacketTrace, rate: u32, metrics: &PacketMetrics) -> Value {
        let packet = data.packet;
        let delta = self
            .clock
            .observe(data.received, packet.device_position, packet.qpc);
        json!({"kind":"packet","packet_index":packet.index,"capture_elapsed_ms":data.elapsed_ms,
            "read_qpc_ticks":clock::read_ticks(),"device_position":packet.device_position,"packet_qpc_100ns":packet.qpc,
            "read_interval_ms":delta.read_interval_ms,"device_delta_frames":delta.device_delta_frames,"timestamp_delta_ms":delta.timestamp_delta_ms,
            "available_frames":packet.available,"used_frames":data.used,"skipped_frames":data.skipped,
            "prefix_silent_frames":data.prefix_silence,"flags":packet.flags,"input_rate":rate,"frames_before":data.frames_before,
            "peaks":metrics.peaks,"sample_fingerprint":metrics.fingerprint.map(|hash|format!("{hash:016x}")),"raw_packet":data.raw,"mapping":data.mapping})
    }
    pub fn endpoint(entry: &mut Option<Value>, progress: Option<CaptureProgress>) {
        if let (Some(entry), Some(p)) = (entry, progress) {
            entry["windows_endpoint_peak"] = json!(p.windows_endpoint_peak);
            entry["windows_endpoint_peak_age_ms"] = json!(p.windows_endpoint_peak_age_ms);
            entry["windows_endpoint_muted"] = json!(p.windows_endpoint_muted);
            entry["windows_endpoint_volume"] = json!(p.windows_endpoint_volume);
            entry["loopback_suspect"] = json!(p.loopback_suspect);
        }
    }
    pub fn complete(
        &mut self,
        entry: Option<Value>,
        sink_started: Instant,
        read_started: Instant,
        delivered: &Result<()>,
    ) {
        if let Some(mut entry) = entry {
            entry["sink_ms"] = json!(sink_started.elapsed().as_secs_f64() * 1000.0);
            entry["processing_ms"] = json!(read_started.elapsed().as_secs_f64() * 1000.0);
            entry["error"] = json!(delivered.as_ref().err().map(ToString::to_string));
            self.emit(entry);
        }
    }
}
pub(super) fn report(
    counters: &Counters,
    input: &Input,
    format: &Format,
    loopback: bool,
    stopped: bool,
) -> Value {
    json!({"mode":"shared","input":input,"format":format,"frames":counters.frames,"packets":counters.packets,
        "signal_frames":counters.signal_frames,"channel_peaks":counters.channel_peaks,"silent_packets":counters.silent_packets,
        "discontinuities":counters.discontinuities,"timestamp_errors":counters.timestamp_errors,"loopback":loopback,
        "intentional_silent_frames":counters.silent_frames,"stopped_by_user":stopped})
}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/diagnostics.rs"]
mod tests;
