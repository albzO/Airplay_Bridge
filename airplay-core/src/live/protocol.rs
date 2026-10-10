//! 原生协议标记解析、会话统计和就绪等待；日志先脱敏，控制解析保留原始文本。
//! Native marker parsing, session statistics and readiness waits; redact logs but parse raw control text.
use super::{GuiEmitter, Result, diagnostics::LogWorker, protocol_log, transport::Backend};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
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

/// stderr 读线程先解析状态/通知就绪，再投递日志及 GUI 事件；不等待日志写入。
/// Parse state/announce readiness before submitting logs and GUI events; never wait for log writes.
pub(super) struct BackendReader {
    pub(super) state: ProtocolState,
    ready: mpsc::Receiver<()>,
    worker: Option<thread::JoinHandle<()>>,
    log: Option<LogWorker<protocol_log::Entry>>,
}
impl BackendReader {
    pub(super) fn start(
        stderr: impl Read + Send + 'static,
        log: impl std::io::Write + Send + 'static,
        privacy: crate::privacy::Redactor,
        detailed_logs: bool,
        emit: Option<GuiEmitter>,
    ) -> std::io::Result<Self> {
        Self::start_with_log(
            stderr,
            protocol_log::start(log, std::io::stdout(), detailed_logs)?,
            privacy,
            emit,
        )
    }
    fn start_with_log(
        stderr: impl Read + Send + 'static,
        log: LogWorker<protocol_log::Entry>,
        privacy: crate::privacy::Redactor,
        emit: Option<GuiEmitter>,
    ) -> std::io::Result<Self> {
        let state = ProtocolState::default();
        let reader_state = state.clone();
        let (ready_tx, ready) = mpsc::channel();
        let log_sender = log.sender();
        let worker = thread::Builder::new().name("airplay-protocol".into()).spawn(move || {
            let state = reader_state;
            let mut announced = false;
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else {
                    break;
                };
                state.record(&line);
                if line.contains("[PROBE] PCM_READY") && !announced {
                    let _ = ready_tx.send(());
                    announced = true;
                }
                let safe_line = crate::log_store::bounded_text(privacy.text(&line));
                let is_fault = key_fault(&line);
                log_sender.record(protocol_log::Entry {
                    safe_line: safe_line.clone(),
                    is_fault,
                    password_needed: line.contains("[PROBE] PASSWORD_NEEDED "),
                });
                if let Some(emit) = &emit {
                    emit(
                        serde_json::json!({"kind":"native","line":line,"safe_line":safe_line,"is_fault":is_fault}),
                    );
                }
            }
        })?;
        Ok(Self {
            state,
            ready,
            worker: Some(worker),
            log: Some(log),
        })
    }

    /// 轮询 50 ms，取消可快速响应；90 秒仍未就绪则沿用现有超时错误。
    /// Poll every 50 ms for responsive cancellation; retain the existing 90-second timeout.
    pub(super) fn wait_ready(&mut self, backend: &mut Backend, stop: &AtomicBool) -> Result<()> {
        self.wait_ready_with(stop, || Ok(backend.try_wait()?.is_some()))
    }
    fn wait_ready_with(
        &mut self,
        stop: &AtomicBool,
        mut backend_exited: impl FnMut() -> Result<bool>,
    ) -> Result<()> {
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
            if backend_exited()? {
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
        if let Some(log) = &mut self.log {
            log.close();
        }
    }
    pub(super) fn finish_log(&mut self, deadline: Instant) -> Option<serde_json::Value> {
        self.log.take().map(|log| log.finish_until(deadline))
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/protocol.rs"]
mod tests;
