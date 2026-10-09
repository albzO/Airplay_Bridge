//! 原生协议标记解析、会话统计和就绪等待；日志先脱敏，控制解析保留原始文本。
//! Native marker parsing, session statistics and readiness waits; redact logs but parse raw control text.
use super::{GuiEmitter, Result, transport::Backend};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use windows::Win32::System::Performance::QueryPerformanceCounter;
#[derive(Clone, Copy, serde::Serialize)]
/// 后端声明的固定播放计划；所有水位计算使用同一 QPC 时基。
/// Backend-declared fixed playback schedule; all water-level calculations share its QPC timebase.
pub(super) struct Clock {
    pub(super) start_qpc: u64,
    pub(super) frequency: u64,
    pub(super) prebuffer_frames: u64,
    pub(super) lead_ms: u64,
}
impl Clock {
    pub(super) fn parse(line: &str) -> Option<Self> {
        if !line.contains("[PROBE] PCM_CLOCK ") {
            return None;
        }
        let field = |name: &str| {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(name)?.parse::<u64>().ok())
        };
        let clock = Self {
            start_qpc: field("start_qpc=")?,
            frequency: field("frequency=")?,
            prebuffer_frames: field("prebuffer_frames=")?,
            lead_ms: field("lead_ms=")?,
        };
        if clock.frequency == 0 {
            None
        } else {
            Some(clock)
        }
    }
    pub(super) fn elapsed(&self) -> Result<f64> {
        let mut counter = 0i64;
        unsafe {
            QueryPerformanceCounter(&mut counter)?;
        }
        Ok((counter as u64).saturating_sub(self.start_qpc) as f64 / self.frequency as f64)
    }
}
fn key_fault(line: &str) -> bool {
    [
        "[ERROR]",
        "[WARN]",
        "ERROR code=",
        "_FAILED",
        "PCM_STALL",
        "PCM_LATE",
        "VOLUME_REJECT",
        "RTSP channel failed",
    ]
    .iter()
    .any(|marker| line.contains(marker))
}
// 保存断管之前的协议故障，以免次生错误覆盖真正原因。
// Preserve the protocol failure that precedes the resulting broken PCM pipe.
fn backend_failure(line: &str) -> Option<String> {
    if line.contains("RTSP channel failed during ")
        || line.contains("[PROBE] PCM_STALL ")
        || line.contains("[PROBE] PCM_LATE ")
        || line.contains("[PROBE] EVENTS_FAILED ")
        || line.contains("[PROBE] GROUP_FAILED ")
    {
        Some(line.to_owned())
    } else {
        None
    }
}

/// 读线程与音频线程共享的协议状态；只保存首个致命故障，避免被断管错误覆盖。
/// Shared reader/audio protocol state; retain the first fatal cause rather than a later pipe failure.
#[derive(Clone, Default)]
pub(super) struct ProtocolState {
    pub(super) transport: Arc<AtomicBool>,
    pub(super) clock: Arc<Mutex<Option<Clock>>>,
    pub(super) packet: Arc<Mutex<Option<String>>>,
    pub(super) member_packets: Arc<Mutex<BTreeMap<String, String>>>,
    pub(super) volume: Arc<Mutex<Option<String>>>,
    pub(super) event: Arc<Mutex<Option<String>>>,
    pub(super) failure: Arc<Mutex<Option<String>>>,
    auth_failure: Arc<Mutex<Option<crate::failure::SessionError>>>,
}
impl ProtocolState {
    fn record(&self, line: &str) {
        if let Some(error) = crate::failure::SessionError::parse(line) {
            *self.auth_failure.lock().unwrap() = Some(error);
        }
        if let Some(failure) = backend_failure(line) {
            let mut first = self.failure.lock().unwrap();
            if first.is_none() {
                *first = Some(failure);
            }
        }
        if line.contains("[PROBE] PACKET_STATS ") {
            *self.packet.lock().unwrap() = Some(line.to_owned());
            if let Some(host) = line
                .split_whitespace()
                .find_map(|s| s.strip_prefix("host="))
            {
                self.member_packets
                    .lock()
                    .unwrap()
                    .insert(host.to_owned(), line.to_owned());
            }
        }
        if line.contains("[PROBE] EVENTS_STATS ") {
            *self.event.lock().unwrap() = Some(line.to_owned());
        }
        if line.contains("[PROBE] VOLUME_CURRENT ") || line.contains("[PROBE] VOLUME_APPLIED ") {
            *self.volume.lock().unwrap() = Some(line.to_owned());
        }
        if let Some(origin) = Clock::parse(line) {
            *self.clock.lock().unwrap() = Some(origin);
        }
        if line.contains("[PROBE] AUDIO_TRANSPORT_OK") {
            self.transport.store(true, Ordering::Relaxed);
        }
    }
}

/// 独立 stderr 读线程。日志限额和最近上下文保持原有行为，PCM_READY 只通知一次。
/// Dedicated stderr reader; preserve log limits/recent context and announce PCM_READY once.
pub(super) struct BackendReader {
    pub(super) state: ProtocolState,
    ready: mpsc::Receiver<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl BackendReader {
    pub(super) fn start(
        stderr: impl Read + Send + 'static,
        mut log: File,
        privacy: crate::privacy::Redactor,
        detailed_logs: bool,
        emit: Option<GuiEmitter>,
    ) -> Self {
        let state = ProtocolState::default();
        let reader_state = state.clone();
        let (ready_tx, ready) = mpsc::channel();
        let worker = thread::spawn(move || {
            let state = reader_state;
            let faults_only = !detailed_logs;
            let mut announced = false;
            let mut recent = std::collections::VecDeque::<String>::new();
            let mut context_written = false;
            let mut log_bytes = 0usize;
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else {
                    break;
                };
                let safe_line = privacy.text(&line);
                println!("{safe_line}");
                let is_fault = key_fault(&line);
                if !faults_only {
                    let _ = writeln!(log, "{safe_line}");
                } else if is_fault && log_bytes < 262144 {
                    if !context_written {
                        let _ = writeln!(log, "[CONTEXT] 最近的会话标记，仅在故障时保存");
                        for item in &recent {
                            let _ = writeln!(log, "{item}");
                            log_bytes += item.len() + 1;
                        }
                        context_written = true;
                    }
                    let bounded: String = safe_line.chars().take(2048).collect();
                    let _ = writeln!(log, "{bounded}");
                    log_bytes += bounded.len() + 1;
                }
                if faults_only {
                    recent.push_back(safe_line.chars().take(1024).collect());
                    if recent.len() > 24 {
                        recent.pop_front();
                    }
                }
                if let Some(emit) = &emit {
                    emit(
                        serde_json::json!({"kind":"native","line":line,"safe_line":safe_line,"is_fault":is_fault}),
                    );
                }
                state.record(&line);
                if line.contains("[PROBE] PASSWORD_NEEDED ") {
                    println!("设备要求 AirPlay 密码，请在本窗口隐藏输入并按回车。");
                }
                if line.contains("[PROBE] PCM_READY") && !announced {
                    let _ = ready_tx.send(());
                    announced = true;
                }
            }
        });
        Self {
            state,
            ready,
            worker: Some(worker),
        }
    }

    /// 轮询 50 ms，取消可快速响应；90 秒仍未就绪则沿用现有超时错误。
    /// Poll every 50 ms for responsive cancellation; retain the existing 90-second timeout.
    pub(super) fn wait_ready(&mut self, backend: &mut Backend, stop: &AtomicBool) -> Result<()> {
        let waiting = Instant::now();
        loop {
            match self.ready.recv_timeout(Duration::from_millis(50)) {
                Ok(()) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(error) = self.state.auth_failure.lock().unwrap().clone() {
                        return Err(error.into());
                    }
                    return Err("后端在音频流准备完成前退出，请查看日志".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if stop.load(Ordering::Relaxed) {
                return Err("已取消建立流".into());
            }
            if waiting.elapsed() > Duration::from_secs(90) {
                return Err("建立流或等待密码超过 90 秒".into());
            }
            if backend.try_wait()?.is_some() {
                self.join();
                if let Some(error) = self.state.auth_failure.lock().unwrap().clone() {
                    return Err(error.into());
                }
                return Err("后端连接失败，请查看日志".into());
            }
        }
    }
    pub(super) fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/protocol.rs"]
mod tests;
