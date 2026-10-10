//! 每次订阅独立的复制缓冲池；只复用存储，保持完整样本与包边界。
//! Per-subscription copy buffers reuse storage while preserving samples and packet boundaries.
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

pub(super) const BUFFER_COUNT: usize = 32;
pub(super) const MAX_BUFFER_SAMPLES: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct Stats {
    created: AtomicU64,
    reused: AtomicU64,
    growths: AtomicU64,
}
impl Stats {
    pub(super) fn snapshot(&self) -> Value {
        json!({"created":self.created.load(Ordering::Relaxed),
            "reused":self.reused.load(Ordering::Relaxed),
            "growths":self.growths.load(Ordering::Relaxed)})
    }
}
pub(super) struct Pool {
    returned: mpsc::Receiver<Vec<f32>>,
    pub(super) stats: Arc<Stats>,
}
impl Pool {
    pub(super) fn new() -> (Self, mpsc::SyncSender<Vec<f32>>) {
        let (tx, returned) = mpsc::sync_channel(BUFFER_COUNT);
        (
            Self {
                returned,
                stats: Arc::new(Stats::default()),
            },
            tx,
        )
    }
    // 由采集端在订阅锁内调用；没有额外的池锁，也不等待消费者归还。
    // Called by capture under the subscription lock; no extra pool mutex or waiting for returns.
    pub(super) fn copy(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut buffer = match self.returned.try_recv() {
            Ok(buffer) => {
                self.stats.reused.fetch_add(1, Ordering::Relaxed);
                buffer
            }
            Err(_) => {
                self.stats.created.fetch_add(1, Ordering::Relaxed);
                Vec::with_capacity(samples.len())
            }
        };
        if buffer.capacity() < samples.len() {
            self.stats.growths.fetch_add(1, Ordering::Relaxed);
        }
        buffer.clear();
        buffer.extend_from_slice(samples);
        buffer
    }
}
// 只保留至多 32 × 64 KiB；大包仍完整交付，用完即释放；池满或已断开也直接释放。
// Retain at most 32 x 64 KiB; deliver large packets intact and release them, full pools or disconnected returns.
pub(super) fn recycle(tx: &mpsc::SyncSender<Vec<f32>>, mut buffer: Vec<f32>) {
    if buffer.capacity() <= MAX_BUFFER_SAMPLES {
        buffer.clear();
        let _ = tx.try_send(buffer);
    }
}

#[cfg(test)]
#[path = "../../../test/core/unit/source/buffers.rs"]
mod tests;
