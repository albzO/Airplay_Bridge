use super::*;
use std::sync::Mutex;

struct MemoryPipe(Arc<Mutex<Vec<u8>>>);
impl Write for MemoryPipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn queue_overflow_and_disconnect_roll_back_pending_bytes() {
    let (tx, rx) = audio_queue::channel();
    let counters = PcmCounters {
        queue: tx.stats.clone(),
        ..PcmCounters::default()
    };
    let (_, recycled) = mpsc::sync_channel(1);
    let sender = PcmSender {
        tx,
        counters: counters.clone(),
        recycled: Mutex::new(recycled),
    };
    sender.send(&[1, -2]).unwrap();
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
    // 接近 640 ms 帧预算，下一块超过预算时必须立即失败。
    // Approach the 640 ms frame budget; a block exceeding it must fail immediately.
    let budget_bytes = (28224 - 1) * 4;
    sender.send(&vec![0; (28224 - 2) * 2]).unwrap();
    assert!(
        sender
            .send(&[3, 4, 5, 6])
            .unwrap_err()
            .to_string()
            .contains("队列已满")
    );
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), budget_bytes);
    assert_eq!(rx.recv().unwrap().value, [1, 0, 254, 255]);
    drop(rx);
    assert!(
        sender
            .send(&[5, 6])
            .unwrap_err()
            .to_string()
            .contains("管道已关闭")
    );
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), budget_bytes);
    assert_eq!(counters.queue.pending_ms(), 0.0);
}

#[test]
fn writer_drains_accepted_stereo_blocks_before_eof() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (sender, writer) = PcmWriter::start(MemoryPipe(bytes.clone()));
    let counters = writer.counters.clone();
    sender.send(&[i16::MIN, i16::MAX]).unwrap();
    sender.send(&[1, -2]).unwrap();
    drop(sender);
    assert_eq!(writer.join().unwrap().unwrap(), 8);
    assert_eq!(*bytes.lock().unwrap(), [0, 128, 255, 127, 1, 0, 254, 255]);
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 0);
    assert_eq!(counters.written_bytes.load(Ordering::Relaxed), 8);
    assert!(counters.last_write_qpc.load(Ordering::Relaxed) > 0);
}

#[test]
fn pipe_failure_is_reported_and_later_sends_fail_without_blocking() {
    struct BrokenPipe;
    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (sender, writer) = PcmWriter::start(BrokenPipe);
    let counters = writer.counters.clone();
    sender.send(&[1, 2]).unwrap();
    assert_eq!(
        writer.join().unwrap().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(sender.send(&[3, 4]).is_err());
    assert_eq!(counters.written_bytes.load(Ordering::Relaxed), 0);
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
    assert_eq!(counters.queue.pending_ms(), 0.0);
}

#[test]
fn stalled_pipe_keeps_duration_reserved_and_resumes_without_losing_pcm() {
    struct GatedPipe {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Write for GatedPipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.entered.try_send(()).is_ok() {
                self.release.recv().unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (entered, started) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let (sender, writer) = PcmWriter::start(GatedPipe {
        entered,
        release: gate,
    });
    let counters = writer.counters.clone();
    let block = vec![1; 441 * 2];
    sender.send(&block).unwrap();
    started
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    for _ in 1..64 {
        sender.send(&block).unwrap();
    }
    assert_eq!(counters.queue.pending_ms(), 640.0);
    assert!(sender.send(&block).is_err());
    // 之后不再阻塞写入；所有接受的 PCM 必须在 EOF 前写出。
    // Subsequent writes proceed; all accepted PCM must be written before EOF.
    drop(started);
    release.send(()).unwrap();
    drop(sender);
    assert_eq!(writer.join().unwrap().unwrap(), 64 * 441 * BYTES_PER_FRAME);
    assert_eq!(counters.queue.pending_ms(), 0.0);
}
