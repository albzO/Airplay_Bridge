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
    let (tx, rx) = mpsc::sync_channel(1);
    let counters = PcmCounters::default();
    let sender = PcmSender {
        tx,
        counters: counters.clone(),
    };
    sender.send(&[1, -2]).unwrap();
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
    assert!(
        sender
            .send(&[3, 4])
            .unwrap_err()
            .to_string()
            .contains("队列已满")
    );
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
    assert_eq!(rx.recv().unwrap(), [1, 0, 254, 255]);
    drop(rx);
    assert!(
        sender
            .send(&[5, 6])
            .unwrap_err()
            .to_string()
            .contains("管道已关闭")
    );
    assert_eq!(counters.pending_bytes.load(Ordering::Relaxed), 4);
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
}
