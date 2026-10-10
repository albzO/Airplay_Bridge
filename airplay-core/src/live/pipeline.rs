//! 单会话音频处理：左右路由、重采样、水位控制、遥测与管线诊断。
//! Per-session audio processing: stereo routing, resampling, water control, telemetry and pipeline traces.
use super::{
    GuiContext, Result,
    diagnostics::DetailLog,
    protocol::ProtocolState,
    transport::{
        BYTES_PER_FRAME, BYTES_PER_MS, FRAMES_PER_MS, OUTPUT_RATE, PcmCounters, PcmSender,
    },
};
use crate::{
    capture::CaptureProgress,
    convert::{Converter, Stats},
    drift::Controller,
};
use std::{
    sync::{Mutex, atomic::Ordering},
    time::{Duration, Instant},
};
use windows::Win32::System::Performance::QueryPerformanceCounter;

// 左右互换使用会话缓冲；未互换时直接借用输入，不复制。
// Swap into a session buffer; otherwise borrow input directly without copying.
fn route<'a>(samples: &'a [f32], swapped: bool, routed: &'a mut Vec<f32>) -> &'a [f32] {
    if !swapped {
        return samples;
    }
    routed.clear();
    routed.extend(samples.chunks_exact(2).flat_map(|f| [f[1], f[0]]));
    routed
}

/// 每次连接独立持有滤波器及控制器；连续采集 Source 不持有这些会话状态。
/// Each connection owns its filter/controller; persistent capture Source owns neither.
pub(super) struct AudioPipeline<'a> {
    gui: Option<&'a GuiContext>,
    stereo: bool,
    seed: u64,
    protocol: &'a ProtocolState,
    progress: &'a Mutex<CaptureProgress>,
    sender: Option<PcmSender>,
    counters: PcmCounters,
    converter: Option<Converter>,
    routed: Vec<f32>,
    controller: Controller,
    captured_frames: u64,
    peaks: [f32; 2],
    ahead_since: Option<Instant>,
    last_update: f64,
    last_log: f64,
    ui_updated: Instant,
    trace_updated: Instant,
    trace: Option<DetailLog>,
    drift_log: Option<DetailLog>,
    capture_diagnostics: bool,
}
pub(super) struct PipelineOptions<'a> {
    pub(super) gui: Option<&'a GuiContext>,
    pub(super) stereo: bool,
    pub(super) seed: u64,
    pub(super) protocol: &'a ProtocolState,
    pub(super) progress: &'a Mutex<CaptureProgress>,
    pub(super) capture_diagnostics: bool,
}
impl<'a> AudioPipeline<'a> {
    pub(super) fn new(
        options: PipelineOptions<'a>,
        sender: PcmSender,
        counters: PcmCounters,
        trace: Option<DetailLog>,
        drift_log: Option<DetailLog>,
    ) -> Self {
        Self {
            gui: options.gui,
            stereo: options.stereo,
            seed: options.seed,
            protocol: options.protocol,
            progress: options.progress,
            sender: Some(sender),
            counters,
            converter: None,
            routed: Vec::new(),
            controller: Controller::new(),
            captured_frames: 0,
            peaks: [0.; 2],
            ahead_since: None,
            last_update: 0.,
            last_log: 0.,
            ui_updated: Instant::now(),
            trace_updated: Instant::now(),
            trace,
            drift_log,
            capture_diagnostics: options.capture_diagnostics,
        }
    }
    /// 回调路由、转换并尝试入队；更新频率保持遥测 250 ms、漂移 1 s、摘要 10 s。
    /// Route, convert and try enqueueing packets; preserve telemetry at 250 ms, drift at 1 s, summaries at 10 s.
    pub(super) fn process(&mut self, samples: &[f32], rate: u32) -> Result<()> {
        self.captured_frames += samples.len() as u64 / 2;
        if self.gui.is_some() {
            for frame in samples.chunks_exact(2) {
                for ch in 0..2 {
                    if frame[ch].is_finite() {
                        self.peaks[ch] = self.peaks[ch].max(frame[ch].abs());
                    }
                }
            }
        }
        if self.converter.is_none() {
            // 重采样器跨包保留滤波器历史和微调状态，不可在每个回调重新创建。
            // Keep filter history and rate correction across packets; never recreate the converter per callback.
            self.converter = Some(Converter::new(rate, self.seed)?);
        }
        let converter = self.converter.as_mut().unwrap();
        let swapped = self.stereo
            && self
                .gui
                .as_ref()
                .is_some_and(|g| g.control.speakers_swapped.load(Ordering::Relaxed));
        let samples = route(samples, swapped, &mut self.routed);
        let mut sink = |pcm: &[i16]| self.sender.as_ref().unwrap().send(pcm);
        let mut converted = converter.push(samples, &mut sink);
        if let Some(origin) = *self.protocol.clock.lock().unwrap() {
            // 控制器至少更新 5 次后，超出目标 120 ms 持续 5 秒才判定无法消化积压。
            // After at least five controller updates, require 120 ms excess for five seconds before failing.
            let water_ms =
                converter.output_frames() as f64 / FRAMES_PER_MS - origin.elapsed()? * 1000.0;
            if self.controller.updates >= 5 && water_ms > self.controller.target_ms + 120.0 {
                let since = self.ahead_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(5) {
                    converted = Err(format!("采集时间线持续超前：水位 {water_ms:.1} ms，目标 {:.1} ms；漂移修正无法消化，已停止以避免队列溢出",self.controller.target_ms).into());
                }
            } else {
                self.ahead_since = None;
            }
        }
        if let Some(trace) = &self.trace {
            // 故障不等一秒采样间隔：保留当时的水位、队列与采集状态，便于还原首因。
            // Record faults immediately rather than waiting a second, preserving water/queue/capture context.
            if self.trace_updated.elapsed() >= Duration::from_secs(1) || converted.is_err() {
                let origin = *self.protocol.clock.lock().unwrap();
                let elapsed = origin.map(|o| o.elapsed()).transpose()?;
                let mut now = 0i64;
                unsafe {
                    QueryPerformanceCounter(&mut now)?;
                }
                let last_write = self.counters.last_write_qpc.load(Ordering::Relaxed);
                trace.record(serde_json::json!({
                    "kind": if converted.is_err() { "fault" } else { "sample" },
                    "qpc": now,
                    "elapsed_seconds": elapsed,
                    "input_rate": rate,
                    "capture_frames": self.captured_frames,
                    "input_audio_seconds": self.captured_frames as f64 / rate as f64,
                    "conversion": converter.stats,
                    "output_audio_seconds": converter.output_frames() as f64 / OUTPUT_RATE,
                    "water_ms": elapsed.map(|s| converter.output_frames() as f64 / FRAMES_PER_MS - s * 1000.0),
                    "pending_pcm_ms": self.counters.pending_bytes.load(Ordering::Relaxed) as f64 / BYTES_PER_MS,
                    "pipe_written_frames": self.counters.written_bytes.load(Ordering::Relaxed) / BYTES_PER_FRAME,
                    "writer_idle_ms": origin.filter(|_| last_write > 0).map(|o| (now as u64).saturating_sub(last_write) as f64 * 1000.0 / o.frequency as f64),
                    "capture": *self.progress.lock().unwrap(),
                    "controller": self.controller,
                    "member_packet_stats": *self.protocol.member_packets.lock().unwrap(),
                    "error": converted.as_ref().err().map(ToString::to_string),
                }));
                if self.capture_diagnostics {
                    if let Some(g) = &self.gui {
                        (g.emit)(serde_json::json!({
                            "kind": "capture_diagnostic",
                            "elapsed_seconds": elapsed,
                            "capture_frames": self.captured_frames,
                            "pending_pcm_ms": self.counters.pending_bytes.load(Ordering::Relaxed) as f64 / BYTES_PER_MS,
                            "water_ms": elapsed.map(|s| converter.output_frames() as f64 / FRAMES_PER_MS - s * 1000.0),
                            "capture": *self.progress.lock().unwrap(),
                            "correction_ppm": self.controller.correction_ppm,
                            "error": converted.as_ref().err().map(ToString::to_string),
                        }));
                    }
                }
                self.trace_updated = Instant::now();
            }
        }
        converted?;
        if let Some(origin) = *self.protocol.clock.lock().unwrap() {
            let elapsed = origin.elapsed()?;
            if let Some(g) = &self.gui {
                if self.ui_updated.elapsed() >= Duration::from_millis(250) {
                    (g.emit)(serde_json::json!({
                        "kind": "telemetry",
                        "elapsed_seconds": elapsed,
                        "input_rate": rate,
                        "capture_frames": self.captured_frames,
                        "output_frames": converter.output_frames(),
                        "water_ms": converter.output_frames() as f64 / FRAMES_PER_MS - elapsed * 1000.0,
                        "lead_ms": origin.lead_ms,
                        "peaks": self.peaks,
                        "controller": self.controller,
                    }));
                    self.peaks = [0.; 2];
                    self.ui_updated = Instant::now();
                }
            }
            if elapsed - self.last_update >= 1.0 {
                // 全管线水位：已生成音频时长减去后端固定播放计划已走过的时间。
                // Whole pipeline water level: all produced samples minus the
                // backend's fixed QPC-paced schedule. Includes queue/pipe/prebuffer.
                let water_ms = converter.output_frames() as f64 / FRAMES_PER_MS - elapsed * 1000.0;
                let ppm = self.controller.update(water_ms, elapsed - self.last_update);
                converter.set_correction_ppm(ppm)?;
                self.last_update = elapsed;
                if elapsed - self.last_log >= 10.0 {
                    let entry = serde_json::json!({
                        "elapsed_seconds": elapsed,
                        "water_ms": water_ms,
                        "target_ms": self.controller.target_ms,
                        "filtered_ms": self.controller.filtered_ms,
                        "correction_ppm": ppm,
                        "lead_ms": origin.lead_ms,
                        "estimated_audio_delay_ms": origin.lead_ms as f64 + water_ms,
                    });
                    if let Some(log) = &self.drift_log {
                        log.record(entry);
                    }
                    self.last_log = elapsed;
                }
            }
        }
        Ok(())
    }
    /// 仅在正常停止时调用，排出已缓存的重采样尾帧；必须在 close_input 之前完成。
    /// Call only on normal stop to drain cached filter tails; complete before close_input.
    pub(super) fn finish_audio(&mut self) -> Result<()> {
        if let Some(converter) = &mut self.converter {
            converter.finish(&mut |pcm| self.sender.as_ref().unwrap().send(pcm))?;
        }
        Ok(())
    }
    /// 先关闭唯一发送端，通知后端 EOF；保留计数及转换状态供报告读取。
    /// Close the sole sender to signal EOF; retain counters and conversion state for the report.
    pub(super) fn close_input(&mut self) {
        self.sender.take();
    }
    pub(super) fn captured_frames(&self) -> u64 {
        self.captured_frames
    }
    pub(super) fn conversion_stats(&self) -> Option<&Stats> {
        self.converter.as_ref().map(|c| &c.stats)
    }
    pub(super) fn controller(&self) -> &Controller {
        &self.controller
    }
    pub(super) fn close_logs(&mut self, capture_result: &Result<serde_json::Value>, stopped: bool) {
        if let Some(trace) = &self.trace {
            trace.record(serde_json::json!({
                "kind": "capture_end",
                "capture_frames": self.captured_frames,
                "conversion": self.conversion_stats(),
                "capture": *self.progress.lock().unwrap(),
                "pcm_queue_peak_ms": self.counters.queue.peak_ms(),
                "pcm_buffers_created": self.counters.buffers_created.load(Ordering::Relaxed),
                "pcm_buffers_reused": self.counters.buffers_reused.load(Ordering::Relaxed),
                "pending_pcm_ms": self.counters.pending_bytes.load(Ordering::Relaxed) as f64 / BYTES_PER_MS,
                "pipe_written_frames": self.counters.written_bytes.load(Ordering::Relaxed) / BYTES_PER_FRAME,
                "error": capture_result.as_ref().err().map(ToString::to_string),
                "stopped_by_user": stopped,
            }));
        }
        if let Some(trace) = &mut self.trace {
            trace.close();
        }
        if let Some(drift) = &mut self.drift_log {
            drift.close();
        }
    }
    pub(super) fn finish_trace(&mut self, deadline: Instant) -> Option<serde_json::Value> {
        self.trace.take().map(|log| log.finish_until(deadline))
    }
    pub(super) fn finish_drift(&mut self, deadline: Instant) -> Option<serde_json::Value> {
        self.drift_log.take().map(|log| log.finish_until(deadline))
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/pipeline.rs"]
mod tests;
