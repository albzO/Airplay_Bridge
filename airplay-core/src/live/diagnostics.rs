//! 有界诊断落盘及日志轮转；磁盘阻塞不能回传到音频线程。
//! Bounded diagnostic writing and rotation; disk stalls must not block the audio thread.
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
// 磁盘写入移到独立线程：慢磁盘不能拖住采集，也不能让长测日志无限占用内存。
// 队列满时计数 dropped；finish 等待已入队记录落盘，报告保留丢记录信息。
// A separate disk worker keeps slow storage off the capture path and bounds logging memory.
// Count dropped entries on overflow; finish flushes queued records and reports those losses.
pub(super) struct DetailLog {
    tx: mpsc::SyncSender<serde_json::Value>,
    dropped: AtomicU64,
    worker: thread::JoinHandle<std::io::Result<()>>,
}
impl DetailLog {
    pub(super) fn start(path: &Path) -> std::io::Result<Self> {
        Self::start_writer(path, 16, u64::MAX, false)
    }
    /// 逐包诊断保留当前段及前三段，每段 8 MiB；队列容纳 1024 条，溢出计数而不阻塞。
    /// Packet traces retain current plus three previous 8 MiB segments; a 1024-entry queue counts overflow.
    pub(super) fn start_capture(path: &Path) -> std::io::Result<Self> {
        Self::start_writer(path, 1024, 8 * 1024 * 1024, true)
    }
    // 队列容量与落盘保留策略独立，调整容量不应隐式改变日志是否轮转。
    // Queue capacity and retention are independent; changing capacity must not silently enable rotation.
    fn start_writer(
        path: &Path,
        capacity: usize,
        limit: u64,
        rolling: bool,
    ) -> std::io::Result<Self> {
        let mut file = File::create(path)?;
        let privacy = crate::privacy::Redactor::from_root(
            path.parent()
                .and_then(Path::parent)
                .unwrap_or(Path::new(".")),
        );
        let path = path.to_owned();
        let (tx, rx) = mpsc::sync_channel(capacity);
        let worker = thread::spawn(move || {
            let mut bytes = 0u64;
            let mut capped = false;
            let mut flushed = Instant::now();
            for entry in rx {
                if capped {
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
                        continue;
                    }
                }
                file.write_all(line.as_bytes())?;
                bytes += line.len() as u64;
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
        });
        Ok(Self {
            tx,
            dropped: AtomicU64::new(0),
            worker,
        })
    }
    pub(super) fn record(&self, entry: serde_json::Value) {
        if self.tx.try_send(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(super) fn finish(self) -> serde_json::Value {
        drop(self.tx);
        let error = match self.worker.join() {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(_) => Some("详细日志线程异常".into()),
        };
        serde_json::json!({"dropped_records":self.dropped.load(Ordering::Relaxed),"error":error})
    }
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
