//! 有界诊断落盘及日志轮转；磁盘阻塞不能回传到音频线程。
//! Bounded diagnostic writing and rotation; disk stalls must not block the audio thread.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
// 磁盘写入移到独立线程：慢磁盘不能拖住采集，也不能让长测日志无限占用内存。
// 队列满时计数 dropped；收尾等待有期限，超时后禁止继续处理队列。
// A separate disk worker keeps slow storage off the capture path and bounds logging memory.
// Count overflow; cleanup has a deadline and abandons queued work after timeout.
pub(crate) struct DetailLog {
    tx: Option<mpsc::SyncSender<serde_json::Value>>,
    dropped: AtomicU64,
    pending: Arc<AtomicU64>,
    abandon: Arc<AtomicBool>,
    completed: mpsc::Receiver<std::io::Result<()>>,
    worker: thread::JoinHandle<()>,
}
impl DetailLog {
    pub(crate) fn start(path: &Path) -> std::io::Result<Self> {
        Self::start_writer(path, 16, u64::MAX, false)
    }
    /// 逐包诊断保留当前段及前三段，每段 8 MiB；队列容纳 1024 条，溢出计数而不阻塞。
    /// Packet traces retain current plus three previous 8 MiB segments; a 1024-entry queue counts overflow.
    pub(super) fn start_capture(path: &Path) -> std::io::Result<Self> {
        Self::start_writer(path, 1024, 8 * 1024 * 1024, true)
    }
    /// 持续 Source 的日志追加及轮转也在工作线程执行；采集只提交快照。
    /// Append/rotate persistent Source logs on the worker; capture only submits snapshots.
    pub(crate) fn start_source(path: &Path) -> std::io::Result<Self> {
        let path = path.to_owned();
        Self::spawn_worker(32, move |rx, pending, abandon| {
            let privacy = redactor(&path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
                let backup = path.with_extension("previous.jsonl");
                if backup.exists() {
                    fs::remove_file(&backup)?;
                }
                fs::rename(&path, backup)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            for entry in rx {
                if abandon.load(Ordering::Acquire) {
                    return Ok(());
                }
                writeln!(file, "{}", privacy.value(&entry))?;
                pending.fetch_sub(1, Ordering::Relaxed);
            }
            file.flush()
        })
    }
    /// CLI 的漂移摘要和可选 JSON 文件共用非阻塞入口；GUI 不向控制台重复输出。
    /// CLI drift summaries and optional JSON files share a nonblocking input; GUI does not echo to stdout.
    pub(super) fn start_drift(path: Option<&Path>, console: bool) -> std::io::Result<Self> {
        let path = path.map(Path::to_owned);
        Self::spawn_worker(16, move |rx, pending, abandon| {
            let mut file = path.map(File::create).transpose()?;
            for entry in rx {
                if abandon.load(Ordering::Acquire) {
                    return Ok(());
                }
                if let Some(file) = &mut file {
                    writeln!(file, "{entry}")?;
                }
                if console {
                    writeln!(
                        std::io::stdout().lock(),
                        "[DRIFT] 缓冲={:.1}ms 目标={:.1}ms 校正={:.1}ppm lead={}ms",
                        entry["water_ms"].as_f64().unwrap_or_default(),
                        entry["target_ms"].as_f64().unwrap_or_default(),
                        entry["correction_ppm"].as_f64().unwrap_or_default(),
                        entry["lead_ms"]
                    )?;
                }
                pending.fetch_sub(1, Ordering::Relaxed);
            }
            if let Some(file) = &mut file {
                file.flush()?;
            }
            Ok(())
        })
    }
    fn spawn_worker(
        capacity: usize,
        work: impl FnOnce(
            mpsc::Receiver<serde_json::Value>,
            Arc<AtomicU64>,
            Arc<AtomicBool>,
        ) -> std::io::Result<()>
        + Send
        + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(capacity);
        let (done_tx, completed) = mpsc::channel();
        let pending = Arc::new(AtomicU64::new(0));
        let abandon = Arc::new(AtomicBool::new(false));
        let worker_pending = pending.clone();
        let worker_abandon = abandon.clone();
        let worker = thread::Builder::new()
            .name("audio-diagnostics".into())
            .spawn(move || {
                let result = work(rx, worker_pending, worker_abandon);
                let _ = done_tx.send(result);
            })?;
        Ok(Self {
            tx: Some(tx),
            dropped: AtomicU64::new(0),
            pending,
            abandon,
            completed,
            worker,
        })
    }
    // 队列容量与落盘保留策略独立，调整容量不应隐式改变日志是否轮转。
    // Queue capacity and retention are independent; changing capacity must not silently enable rotation.
    fn start_writer(
        path: &Path,
        capacity: usize,
        limit: u64,
        rolling: bool,
    ) -> std::io::Result<Self> {
        let path = path.to_owned();
        Self::spawn_worker(capacity, move |rx, pending, abandon| {
            let privacy = redactor(&path);
            let mut file = File::create(&path)?;
            let mut bytes = 0u64;
            let mut capped = false;
            let mut flushed = Instant::now();
            for entry in rx {
                if abandon.load(Ordering::Acquire) {
                    return Ok(());
                }
                if capped {
                    pending.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let line = format!("{}\n", privacy.value(&entry));
                if bytes + line.len() as u64 > limit {
                    if rolling {
                        file.flush()?;
                        drop(file);
                        rotate_capture_logs(&path)?;
                        file = File::create(&path)?;
                        bytes = 0;
                    } else {
                        capped = true;
                        pending.fetch_sub(1, Ordering::Relaxed);
                        continue;
                    }
                }
                file.write_all(line.as_bytes())?;
                bytes += line.len() as u64;
                pending.fetch_sub(1, Ordering::Relaxed);
                if flushed.elapsed() >= Duration::from_millis(250) {
                    file.flush()?;
                    flushed = Instant::now();
                }
            }
            file.flush()?;
            if capped {
                return Err(std::io::Error::other(format!(
                    "详细诊断达到 {limit} 字节上限，后续记录已省略"
                )));
            }
            Ok(())
        })
    }
    pub(crate) fn record(&self, entry: serde_json::Value) {
        self.pending.fetch_add(1, Ordering::Relaxed);
        if self
            .tx
            .as_ref()
            .is_none_or(|tx| tx.try_send(entry).is_err())
        {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn finish(self) -> serde_json::Value {
        self.finish_until(Instant::now() + Duration::from_millis(250))
    }
    // 所有发送端先一起关闭，让健康线程并行排空，再等待共享期限。
    // Close all senders first so healthy workers can drain concurrently before the shared deadline.
    pub(super) fn close(&mut self) {
        self.tx.take();
    }
    /// 多个日志共用截止时间，不能每个线程重新获得一份等待预算。
    /// Share one deadline across logs rather than granting each worker a fresh wait budget.
    pub(super) fn finish_until(mut self, deadline: Instant) -> serde_json::Value {
        self.close();
        let result = self
            .completed
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let timed_out = matches!(result, Err(mpsc::RecvTimeoutError::Timeout));
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(mpsc::RecvTimeoutError::Disconnected) => Some("详细日志线程异常".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => Some("日志收尾超时，未完成记录可能丢失".into()),
        };
        // 超时无法取消正在进行的系统 I/O；脱离等待后，线程恢复时不再处理积压。
        // Timeout cannot cancel in-flight system I/O; detach and stop queued work if the worker recovers.
        self.abandon.store(true, Ordering::Release);
        drop(self.worker);
        serde_json::json!({"dropped_records":self.dropped.load(Ordering::Relaxed),
            "pending_records":self.pending.load(Ordering::Relaxed),"timed_out":timed_out,"error":error})
    }
}
fn redactor(path: &Path) -> crate::privacy::Redactor {
    crate::privacy::Redactor::from_root(
        path.parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new(".")),
    )
}
// 仅保留本次会话的当前日志和前三段旧日志。
// Keep current plus three previous segments, all local to this session.
fn rotate_capture_logs(path: &Path) -> std::io::Result<()> {
    let segment = |n| path.with_extension(format!("previous-{n}.jsonl"));
    if segment(3).exists() {
        fs::remove_file(segment(3))?;
    }
    for n in (1..3).rev() {
        if segment(n).exists() {
            fs::rename(segment(n), segment(n + 1))?;
        }
    }
    fs::rename(path, segment(1))
}

#[cfg(test)]
#[path = "../../../test/core/unit/live/diagnostics.rs"]
mod tests;
