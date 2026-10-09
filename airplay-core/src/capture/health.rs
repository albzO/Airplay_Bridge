//! 播放回环启动检查：首次稳定后永久结束判断，避免暂停播放误触发重建。
//! Playback loopback startup check: permanently finish after stability to avoid reopening on playback pause.
use std::{
    fmt,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Media::Audio::{
            Endpoints::{IAudioEndpointVolume, IAudioMeterInformation},
            IMMDeviceEnumerator, MMDeviceEnumerator,
        },
        System::Com::{CLSCTX_ALL, CoCreateInstance},
    },
    core::PCWSTR,
};

#[derive(Clone, Copy)]
pub(super) struct PlaybackReading {
    pub(super) peak: Option<f32>,
    pub(super) muted: Option<bool>,
    pub(super) volume: Option<f32>,
}
impl PlaybackReading {
    fn audible(self) -> bool {
        self.peak
            .is_some_and(|peak| peak.is_finite() && peak > 0.01)
            && self.muted == Some(false)
            && self
                .volume
                .is_some_and(|volume| volume.is_finite() && volume > 0.0)
    }
}

/// 电平在端点音量调整前测量，必须同时检查静音和零音量，避免把正常静音当故障。
/// Endpoint meters precede endpoint attenuation; also check mute/zero volume to avoid false faults.
pub(super) struct PlaybackMonitor {
    meter: IAudioMeterInformation,
    volume: Option<IAudioEndpointVolume>,
    reading: Option<(Instant, PlaybackReading)>,
}
impl PlaybackMonitor {
    pub(super) fn open(endpoint: &str) -> super::Result<Self> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let wide: Vec<u16> = endpoint.encode_utf16().chain(Some(0)).collect();
        let device = unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr()))? };
        Ok(Self {
            meter: unsafe { device.Activate(CLSCTX_ALL, None)? },
            volume: unsafe { device.Activate(CLSCTX_ALL, None) }.ok(),
            reading: None,
        })
    }
    pub(super) fn read(&mut self, now: Instant) -> (PlaybackReading, f64) {
        if self
            .reading
            .is_none_or(|(updated, _)| now.duration_since(updated) >= Duration::from_millis(100))
        {
            self.reading = Some((
                now,
                PlaybackReading {
                    peak: unsafe { self.meter.GetPeakValue() }.ok(),
                    muted: self
                        .volume
                        .as_ref()
                        .and_then(|volume| unsafe { volume.GetMute() }.ok())
                        .map(|muted| muted.as_bool()),
                    volume: self
                        .volume
                        .as_ref()
                        .and_then(|volume| unsafe { volume.GetMasterVolumeLevelScalar() }.ok()),
                },
            ));
        }
        let (updated, reading) = self.reading.unwrap();
        (reading, now.duration_since(updated).as_secs_f64() * 1000.0)
    }
}

#[derive(Default)]
pub(super) struct LoopbackHealth {
    mismatch_since: Option<Instant>,
    healthy_since: Option<Instant>,
    startup_complete: bool,
}
impl LoopbackHealth {
    /// 检查全部原始声道，而非用户映射后的左右声道；后者可能合法地选择静音声道。
    /// 仅在启动阶段判断：连续 500 ms 无矛盾后永久结束检查，暂停/恢复不重新开启。
    /// 首个启动矛盾包暂停就绪，持续 500 ms 才请求重建；正常静音也可完成启动。
    /// Check all raw channels, not mapped stereo which may intentionally select silent channels.
    /// Startup only: 500 ms without contradiction permanently completes the check; pause/resume never rearms it.
    /// Suspend readiness on the first startup mismatch; reopen after 500 ms. Valid silence also completes startup.
    pub(super) fn observe(
        &mut self,
        now: Instant,
        reading: PlaybackReading,
        raw_nonzero: bool,
    ) -> bool {
        if self.startup_complete {
            return false;
        }
        if reading.audible() && !raw_nonzero {
            self.mismatch_since.get_or_insert(now);
            self.healthy_since = None;
        } else {
            self.mismatch_since = None;
            let since = *self.healthy_since.get_or_insert(now);
            if now.duration_since(since) >= Duration::from_millis(500) {
                self.startup_complete = true;
            }
        }
        self.mismatch_since.is_some()
    }
    pub(super) fn stalled(&self, now: Instant) -> bool {
        self.mismatch_since
            .is_some_and(|since| now.duration_since(since) >= Duration::from_millis(500))
    }
}

/// 可恢复的采集故障类型；来源监督器只对此故障重建，不能把任意错误当成重试理由。
/// Recoverable capture fault; the source supervisor reopens only for this type, never arbitrary errors.
#[derive(Debug)]
pub(crate) struct LoopbackStalled;
impl fmt::Display for LoopbackStalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "播放设备有输出但 loopback 原始音频持续全零，采集客户端需要重新初始化"
        )
    }
}
impl std::error::Error for LoopbackStalled {}

#[cfg(test)]
#[path = "../../../test/core/unit/capture/health.rs"]
mod tests;
