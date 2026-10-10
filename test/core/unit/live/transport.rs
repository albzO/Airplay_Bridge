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

// The test executable is also a device-free child process. Its environment is private to Command.
#[test]
fn subprocess_fixture() {
    match std::env::var("AIRPLAY_STOP_FIXTURE").as_deref() {
        Ok("hang") => thread::sleep(std::time::Duration::from_secs(60)),
        Ok("eof") => {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut io::stdin(), &mut bytes).unwrap();
        }
        _ => {}
    }
}
fn fixture_backend(mode: &str) -> Backend {
    let child = Command::new(std::env::current_exe().unwrap())
        .creation_flags(0x08000000)
        .args([
            "--exact",
            "live::transport::tests::subprocess_fixture",
            "--nocapture",
        ])
        .env("AIRPLAY_STOP_FIXTURE", mode)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    Backend::from_child(child).unwrap()
}

#[test]
fn backend_stops_gracefully_on_eof_and_terminates_an_unresponsive_child() {
    for mode in ["eof", "hang"] {
        let mut backend = fixture_backend(mode);
        drop(backend.stdin().unwrap());
        let start = Instant::now();
        let deadline = start + std::time::Duration::from_secs(3);
        let grace = if mode == "eof" {
            deadline - std::time::Duration::from_millis(500)
        } else {
            start + std::time::Duration::from_millis(40)
        };
        let (status, error, forced) = backend.stop_until(deadline, grace);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(forced, mode == "hang");
        assert_eq!(status.unwrap().success(), mode == "eof");
        assert!(start.elapsed() < std::time::Duration::from_secs(4));
    }
}

#[test]
fn job_guard_terminates_its_child_without_waiting_in_drop() {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let backend = fixture_backend("hang");
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, backend.child.id()) }.unwrap();
    let start = Instant::now();
    drop(backend);
    let elapsed = start.elapsed();
    let result = unsafe { WaitForSingleObject(process, 3000) };
    let _ = unsafe { CloseHandle(process) };
    assert!(elapsed < std::time::Duration::from_millis(500));
    assert_eq!(result, WAIT_OBJECT_0);
}

#[test]
fn real_stderr_and_pcm_pipes_release_after_forced_backend_stop() {
    use crate::live::protocol::BackendReader;
    let mut backend = fixture_backend("hang");
    let mut reader = BackendReader::start(
        backend.stderr().unwrap(),
        io::sink(),
        crate::privacy::Redactor::default(),
        false,
        None,
    )
    .unwrap();
    let (sender, writer) = PcmWriter::start(backend.stdin().unwrap());
    sender.send(&vec![1; 28224 * 2]).unwrap();
    drop(sender);
    let start = Instant::now();
    let deadline = start + std::time::Duration::from_secs(3);
    let (status, error, forced) =
        backend.stop_until(deadline, start + std::time::Duration::from_millis(40));
    assert!(status.is_some() && error.is_none() && forced);
    assert!(writer.join_until(deadline).is_err());
    reader.join_until(deadline).unwrap();
    let state = serde_json::to_value(&*reader.state.reader.lock().unwrap()).unwrap();
    assert_eq!(state["state"], "eof");
    assert!(start.elapsed() < std::time::Duration::from_secs(4));
}

#[test]
fn stalled_generic_writer_returns_at_deadline_and_does_not_drain_after_cancellation() {
    struct BlockedPipe {
        entered: Option<mpsc::Sender<()>>,
        gate: mpsc::Receiver<()>,
        written: Arc<AtomicU64>,
    }
    impl Write for BlockedPipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                let _ = self.gate.recv();
            }
            self.written
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (release, gate) = mpsc::channel();
    let (entered, began) = mpsc::channel();
    let written = Arc::new(AtomicU64::new(0));
    let (sender, writer) = PcmWriter::start(BlockedPipe {
        entered: Some(entered),
        gate,
        written: written.clone(),
    });
    sender.send(&[1, 2]).unwrap();
    began
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    sender.send(&[3, 4]).unwrap();
    drop(sender);
    let start = Instant::now();
    assert!(
        writer
            .join_until(start + std::time::Duration::from_millis(40))
            .unwrap_err()
            .to_string()
            .contains("STOP_TIMEOUT")
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    drop(release);
    let until = Instant::now() + std::time::Duration::from_secs(2);
    while written.load(Ordering::Relaxed) == 0 && Instant::now() < until {
        thread::yield_now();
    }
    assert_eq!(written.load(Ordering::Relaxed), 4);
}
