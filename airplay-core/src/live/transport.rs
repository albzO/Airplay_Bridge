//! 原生进程生命周期及有界 PCM 发送；采集线程只尝试入队，不等待管道写入。
//! Native process lifetime and bounded PCM delivery; capture only tries enqueueing, never pipe writes.
use super::{GuiContext, Result};
use crate::audio_queue::{self, AudioSender, QueueError, QueueStats};
use std::{
    io::{self, Write},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};
use windows::Win32::{
    Foundation::HANDLE,
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Performance::QueryPerformanceCounter,
    },
};

// 一帧含左右两个 i16：4 字节；统一单位换算，防止把采样数当帧数或把字节当毫秒。
// One frame contains two i16 channels: four bytes. Centralize conversions between samples, frames and ms.
pub(super) const OUTPUT_RATE: f64 = 44100.0;
pub(super) const FRAMES_PER_MS: f64 = OUTPUT_RATE / 1000.0;
pub(super) const BYTES_PER_FRAME: u64 = 4;
pub(super) const BYTES_PER_MS: f64 = FRAMES_PER_MS * BYTES_PER_FRAME as f64;

/// 已校验会话的后端参数；密码仍通过命名管道传输，不加入命令参数。
/// Backend options for a validated session; passwords stay in the named pipe, not command arguments.
pub(super) struct BackendOptions<'a> {
    pub(super) executable: PathBuf,
    pub(super) gui: Option<&'a GuiContext>,
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) name: &'a str,
    pub(super) txt: &'a str,
    pub(super) identity: &'a str,
    pub(super) active_remote: &'a str,
    pub(super) volume_control_port: u16,
    pub(super) latency_ms: u32,
    pub(super) buffer_ms: u32,
    pub(super) detailed_logs: bool,
    pub(super) peer_args: &'a [String],
}

/// 进程守卫：正常结束保留退出状态，错误返回或提前退出时关闭 Job，终止子进程树。
/// Process guard: preserve normal exits; closing the job terminates the child tree without waiting.
pub(super) struct Backend {
    child: Child,
    _job: OwnedHandle,
}
impl Backend {
    pub(super) fn spawn(options: BackendOptions<'_>) -> Result<Self> {
        let child = Command::new(options.executable)
            .creation_flags(if options.gui.is_some() { 0x08000000 } else { 0 })
            .args([
                "--host",
                options.host,
                "--port",
                &options.port.to_string(),
                "--name",
                options.name,
                "--txt",
                options.txt,
                "--timing",
                "ptp",
                "--hold-seconds",
                "0",
                "--password-auto",
                "--pcm-stdin",
                "--identity",
                options.identity,
                "--active-remote",
                options.active_remote,
                "--volume-control-port",
                &options.volume_control_port.to_string(),
                "--latency-ms",
                &options.latency_ms.to_string(),
                "--buffer-ms",
                &options.buffer_ms.to_string(),
            ])
            .args(options.peer_args)
            .args(if options.detailed_logs {
                vec!["--debug"]
            } else {
                Vec::new()
            })
            .args(
                options
                    .gui
                    .map(|g| {
                        let mut args = vec!["--password-pipe".to_owned(), g.password_pipe.clone()];
                        if g.password_first {
                            args.push("--password-pipe-first".into());
                        }
                        if g.peer_password_first {
                            args.push("--peer-password-first".into());
                        }
                        args
                    })
                    .unwrap_or_default(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .spawn()?;
        Self::from_child(child)
    }
    fn from_child(mut child: Child) -> Result<Self> {
        let setup = (|| -> windows::core::Result<OwnedHandle> {
            let raw = unsafe { CreateJobObjectW(None, None)? };
            let job = unsafe { OwnedHandle::from_raw_handle(raw.0) };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    raw,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                )?;
                AssignProcessToJobObject(raw, HANDLE(child.as_raw_handle()))?;
            }
            Ok(job)
        })();
        match setup {
            Ok(job) => Ok(Self { child, _job: job }),
            Err(error) => {
                let _ = child.kill();
                Err(format!("后端生命周期 / 配置 Windows Job Object：{error}").into())
            }
        }
    }
    pub(super) fn stderr(&mut self) -> Result<ChildStderr> {
        self.child
            .stderr
            .take()
            .ok_or_else(|| "缺少后端日志管道".into())
    }
    pub(super) fn stdin(&mut self) -> Result<ChildStdin> {
        self.child
            .stdin
            .take()
            .ok_or_else(|| "缺少 PCM 管道".into())
    }
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    pub(super) fn kill(&mut self) -> io::Result<()> {
        unsafe { TerminateJobObject(HANDLE(self._job.as_raw_handle()), 1) }
            .map_err(|error| io::Error::from_raw_os_error((error.code().0 as u32 & 0xffff) as i32))
    }
    /// Keep polling after termination, but never call Child::wait on a stop path.
    pub(super) fn stop_until(
        &mut self,
        deadline: Instant,
        grace: Instant,
    ) -> (Option<ExitStatus>, Option<String>, bool) {
        let mut forced = false;
        loop {
            match self.try_wait() {
                Ok(Some(status)) => return (Some(status), None, forced),
                Err(error) => return (None, Some(format!("查询后端退出状态：{error}")), forced),
                Ok(None) => {}
            }
            let now = Instant::now();
            if now >= grace && !forced {
                forced = true;
                if let Err(error) = self.kill() {
                    return (None, Some(format!("终止后端进程：{error}")), forced);
                }
            }
            if now >= deadline {
                return (
                    None,
                    Some("[STOP_TIMEOUT] 后端未在统一停止期限内退出".into()),
                    forced,
                );
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(now)
                    .min(std::time::Duration::from_millis(5)),
            );
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            // Closing the job also terminates descendants retaining pipe handles. No blocking wait here.
        }
    }
}

/// 队列与写线程的累计计数，用于遥测和诊断；不是另一套播放时钟。
/// Queue/writer counters for telemetry and diagnostics, not a second playback clock.
#[derive(Clone, Default)]
pub(super) struct PcmCounters {
    pub(super) pending_bytes: Arc<AtomicU64>,
    pub(super) written_bytes: Arc<AtomicU64>,
    pub(super) last_write_qpc: Arc<AtomicU64>,
    pub(super) queue: Arc<QueueStats>,
    pub(super) buffers_created: Arc<AtomicU64>,
    pub(super) buffers_reused: Arc<AtomicU64>,
}
pub(super) struct PcmSender {
    tx: AudioSender<Vec<u8>>,
    counters: PcmCounters,
    recycled: Mutex<mpsc::Receiver<Vec<u8>>>,
}
impl PcmSender {
    /// PCM 转为 S16LE，入队失败撤销待发送计数；队列满立即报错，保护采集时序。
    /// Encode S16LE and roll back pending bytes on enqueue failure; overflow fails without blocking capture.
    pub(super) fn send(&self, pcm: &[i16]) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        if pcm.len() % 2 != 0 {
            return Err("PCM 必须包含完整左右声道帧".into());
        }
        let length = pcm.len() as u64 * 2;
        self.counters
            .pending_bytes
            .fetch_add(length, Ordering::Relaxed);
        self.tx
            .try_send(pcm.len() / 2, OUTPUT_RATE as u32, || {
                let mut bytes = match self.recycled.lock().unwrap().try_recv() {
                    Ok(buffer) => {
                        self.counters.buffers_reused.fetch_add(1, Ordering::Relaxed);
                        buffer
                    }
                    Err(_) => {
                        self.counters
                            .buffers_created
                            .fetch_add(1, Ordering::Relaxed);
                        Vec::with_capacity(length as usize)
                    }
                };
                bytes.clear();
                bytes.extend(pcm.iter().flat_map(|s| s.to_le_bytes()));
                bytes
            })
            .map_err(|err| {
                self.counters
                    .pending_bytes
                    .fetch_sub(length, Ordering::Relaxed);
                match err {
                    QueueError::Full => "PCM 队列已满，发送跟不上采集；已停止",
                    QueueError::Disconnected => "后端 PCM 管道已关闭",
                    QueueError::InvalidFormat => "PCM 音频格式无效",
                }
            })?;
        Ok(())
    }
}
/// 按输出帧数限制为 640 ms（含写入中的块）；关闭发送端以 EOF 请求收尾。
/// Bound output frames to 640 ms including the active write; dropping the sender requests EOF.
pub(super) struct PcmWriter {
    pub(super) counters: PcmCounters,
    worker: Option<thread::JoinHandle<io::Result<u64>>>,
    cancel: Arc<AtomicBool>,
}
impl PcmWriter {
    pub(super) fn start(mut pipe: impl Write + Send + 'static) -> (PcmSender, Self) {
        let (tx, rx) = audio_queue::channel::<Vec<u8>>();
        let (recycle_tx, recycle_rx) = mpsc::sync_channel(32);
        let counters = PcmCounters {
            queue: tx.stats.clone(),
            ..PcmCounters::default()
        };
        let output = counters.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let worker = thread::spawn(move || -> io::Result<u64> {
            let mut bytes = 0;
            for mut block in rx {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::other("[STOP_TIMEOUT] PCM 写入已取消"));
                }
                pipe.write_all(&block.value)?;
                bytes += block.value.len() as u64;
                output
                    .pending_bytes
                    .fetch_sub(block.value.len() as u64, Ordering::Relaxed);
                output.written_bytes.store(bytes, Ordering::Relaxed);
                let mut counter = 0i64;
                if unsafe { QueryPerformanceCounter(&mut counter) }.is_ok() {
                    output
                        .last_write_qpc
                        .store(counter as u64, Ordering::Relaxed);
                }
                // 只回收有限个缓冲；池满时释放，不在写线程等待转换线程。
                // Recycle only a bounded number of buffers; a full pool drops them without waiting.
                let _ = recycle_tx.try_send(std::mem::take(&mut block.value));
            }
            // EOF 请求尾部静音和 TEARDOWN，由原生后端完成协议收尾。
            // EOF requests tail silence and TEARDOWN in the native backend.
            drop(pipe);
            Ok(bytes)
        });
        (
            PcmSender {
                tx,
                counters: counters.clone(),
                recycled: Mutex::new(recycle_rx),
            },
            Self {
                counters,
                worker: Some(worker),
                cancel,
            },
        )
    }
    pub(super) fn join_until(mut self, deadline: Instant) -> io::Result<u64> {
        crate::shutdown::join_until(&mut self.worker, deadline, false)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("PCM 写线程缺失"))?
    }
    #[cfg(test)]
    pub(super) fn join(self) -> Result<io::Result<u64>> {
        Ok(self.join_until(Instant::now() + std::time::Duration::from_secs(2)))
    }
}
impl Drop for PcmWriter {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = &self.worker {
            crate::shutdown::cancel(worker);
        }
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/transport.rs"]
mod tests;
