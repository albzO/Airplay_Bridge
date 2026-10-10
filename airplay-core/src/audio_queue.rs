//! 两级音频队列共用 640 ms 时长预算，另设 256 块硬上限限制极小包的元数据。
//! 预算包含消费者正在处理的块；RAII 在成功、失败和丢弃路径上归还额度。
//! Both audio queues share a 640 ms duration policy plus a 256-block cap for tiny-packet metadata.
//! The budget includes in-flight consumer blocks; RAII releases it on success, failure and discard.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

pub(crate) const AUDIO_BUDGET_MS: u64 = 640;
pub(crate) const BLOCK_LIMIT: usize = 256;
const BUDGET_NS: u64 = AUDIO_BUDGET_MS * 1_000_000;

#[derive(Default)]
pub(crate) struct QueueStats {
    pending_ns: AtomicU64,
    peak_ns: AtomicU64,
}
impl QueueStats {
    pub fn pending_ms(&self) -> f64 {
        self.pending_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }
    pub fn peak_ms(&self) -> f64 {
        self.peak_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }
}
struct Reservation {
    stats: Arc<QueueStats>,
    duration_ns: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.stats
            .pending_ns
            .fetch_sub(self.duration_ns, Ordering::Relaxed);
    }
}
pub(crate) struct Queued<T> {
    pub value: T,
    _reservation: Reservation,
}
#[derive(Debug, PartialEq)]
pub(crate) enum QueueError {
    Full,
    Disconnected,
    InvalidFormat,
}
pub(crate) struct AudioSender<T> {
    tx: mpsc::SyncSender<Queued<T>>,
    pub stats: Arc<QueueStats>,
}
pub(crate) fn channel<T>() -> (AudioSender<T>, mpsc::Receiver<Queued<T>>) {
    let (tx, rx) = mpsc::sync_channel(BLOCK_LIMIT);
    (
        AudioSender {
            tx,
            stats: Arc::new(QueueStats::default()),
        },
        rx,
    )
}
impl<T> AudioSender<T> {
    /// 先按帧数/采样率预留时长，再执行分配/复制；不阻塞，队列满时撤销预留。
    /// Reserve by frames/rate before allocation/copy; never block and roll back a failed enqueue.
    pub fn try_send(
        &self,
        frames: usize,
        rate: u32,
        make: impl FnOnce() -> T,
    ) -> Result<(), QueueError> {
        if rate == 0 || frames == 0 {
            return Err(QueueError::InvalidFormat);
        }
        let duration_ns = (frames as u128 * 1_000_000_000).div_ceil(rate as u128);
        if duration_ns > BUDGET_NS as u128 {
            return Err(QueueError::Full);
        }
        let duration_ns = duration_ns as u64;
        let previous = self
            .stats
            .pending_ns
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
                (pending <= BUDGET_NS - duration_ns).then_some(pending + duration_ns)
            })
            .map_err(|_| QueueError::Full)?;
        self.stats
            .peak_ns
            .fetch_max(previous + duration_ns, Ordering::Relaxed);
        let reservation = Reservation {
            stats: self.stats.clone(),
            duration_ns,
        };
        self.tx
            .try_send(Queued {
                value: make(),
                _reservation: reservation,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => QueueError::Full,
                mpsc::TrySendError::Disconnected(_) => QueueError::Disconnected,
            })
    }
}

#[cfg(test)]
#[path = "../../test/core/unit/audio_queue.rs"]
mod tests;
