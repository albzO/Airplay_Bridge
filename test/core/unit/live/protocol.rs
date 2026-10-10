use super::*;
use std::io::{self, Cursor, Write};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

struct StalledWriter {
    entered: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
    exited: mpsc::Sender<()>,
    buffer: Buffer,
}
impl Write for StalledWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
            // 发送端随测试作用域销毁，断言失败也能解除故意阻塞。
            // Dropping the fixture sender releases the intentional stall even after an assertion failure.
            let _ = self.release.recv();
        }
        self.buffer.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for StalledWriter {
    fn drop(&mut self) {
        let _ = self.exited.send(());
    }
}

// 首行触发写入后，才放行余下协议行，确保实际 writer 已阻塞而非依赖调度顺序。
// Release subsequent markers only once the first line has blocked the actual writer.
struct GatedInput {
    first: Cursor<Vec<u8>>,
    entered: Option<mpsc::Receiver<()>>,
    rest: Cursor<Vec<u8>>,
    eof_release: Option<mpsc::Receiver<()>>,
}
impl Read for GatedInput {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.first.read(bytes)?;
        if count != 0 {
            return Ok(count);
        }
        if let Some(entered) = self.entered.take() {
            entered
                .recv_timeout(Duration::from_secs(2))
                .map_err(io::Error::other)?;
        }
        let count = self.rest.read(bytes)?;
        if count == 0 {
            if let Some(release) = self.eof_release.take() {
                let _ = release.recv();
            }
        }
        Ok(count)
    }
}

#[test]
fn reader_preserves_raw_state_but_redacts_logs_and_emitted_display_lines() {
    let path = std::env::temp_dir().join(format!("airplay-reader-{}.log", std::process::id()));
    let lines = [
        "[PROBE] PCM_READY",
        "[PROBE] PCM_READY",
        "[PROBE] PCM_CLOCK start_qpc=1 frequency=10000000 prebuffer_frames=5984 lead_ms=300",
        "[PROBE] PCM_CLOCK start_qpc=2 frequency=0 prebuffer_frames=5984 lead_ms=300",
        "[PROBE] PACKET_STATS host=192.0.2.10 sent=10",
        "[PROBE] PACKET_STATS host=192.0.2.11 sent=20",
        "[PROBE] VOLUME_CURRENT percent=50",
        "[PROBE] EVENTS_STATS received=3",
        "[PROBE] PCM_STALL timeout_ms=1000 password=fictional-password",
        "[PROBE] PCM_LATE deadline_ms=1234",
        "[PROBE] ERROR code=PASSWORD_REJECTED exit=10 host=fixture phase=pairing",
        "[PROBE] AUDIO_TRANSPORT_OK",
    ];
    let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let received = events.clone();
    let mut reader = BackendReader::start(
        std::io::Cursor::new(lines.join("\n")),
        File::create(&path).unwrap(),
        crate::privacy::Redactor::default(),
        false,
        Some(Arc::new(move |event| received.lock().unwrap().push(event))),
    )
    .unwrap();
    reader.join();
    assert!(
        reader
            .finish_log(Instant::now() + Duration::from_secs(1))
            .unwrap()["error"]
            .is_null()
    );
    assert!(reader.ready.try_recv().is_ok());
    assert!(reader.ready.try_recv().is_err());
    let state = &reader.state;
    assert!(state.transport.load(Ordering::Relaxed));
    assert_eq!(state.clock.lock().unwrap().unwrap().start_qpc, 1);
    assert_eq!(state.member_packets.lock().unwrap().len(), 2);
    assert!(
        state
            .packet
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("sent=20")
    );
    assert!(
        state
            .volume
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("percent=50")
    );
    assert!(
        state
            .event
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("received=3")
    );
    assert!(
        state
            .failure
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("PCM_STALL")
    );
    assert_eq!(
        state.auth_failure.lock().unwrap().as_ref().unwrap().exit,
        10
    );
    let log = std::fs::read_to_string(&path).unwrap();
    assert!(log.contains("[CONTEXT]") && log.contains("PCM_READY") && log.contains("PCM_STALL"));
    assert!(!log.contains("fictional-password") && !log.contains("192.0.2.10"));
    let events = events.lock().unwrap();
    assert_eq!(events.len(), lines.len());
    assert!(
        events
            .iter()
            .any(|e| e["line"].as_str().unwrap().contains("192.0.2.10"))
    );
    assert!(events.iter().all(|e| {
        !e["safe_line"]
            .as_str()
            .unwrap()
            .contains("fictional-password")
    }));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn backend_failure_distinguishes_disconnect_from_success_and_tolerated_miss() {
    assert!(backend_failure("[ERROR] [AP2] RTSP channel failed during POST /feedback read after 0ms: connection reset; terminating native session").is_some());
    assert!(backend_failure("[PROBE] PCM_STALL timeout_ms=1000").is_some());
    assert!(backend_failure("[PROBE] PCM_LATE deadline_ms=1234").is_some());
    assert!(backend_failure("[PROBE] EVENTS_FAILED reason=authentication").is_some());
    assert!(
        backend_failure("[WARN] POST /feedback keepalive miss 1/3; tolerating transient failure")
            .is_none()
    );
    assert!(backend_failure("[PROBE] PCM_EOF frames=44100").is_none());
    assert!(backend_failure("[PROBE] AUDIO_TRANSPORT_OK").is_none());
}

#[test]
fn clock_marker_uses_shared_qpc_and_rejects_invalid_frequency() {
    let mut now = 0;
    unsafe {
        QueryPerformanceCounter(&mut now).unwrap();
    }
    let line = format!(
        "[PROBE] PCM_CLOCK start_qpc={now} frequency=10000000 prebuffer_frames=5984 lead_ms=1000"
    );
    let clock = Clock::parse(&line).unwrap();
    assert!(clock.elapsed().unwrap() < 1.0);
    assert!(
        Clock::parse("[PROBE] PCM_CLOCK start_qpc=1 frequency=0 prebuffer_frames=1 lead_ms=1")
            .is_none()
    );
}

#[test]
fn stalled_file_or_console_cannot_block_readiness_faults_or_reader_cleanup() {
    for stall_file in [true, false] {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (exited_tx, exited_rx) = mpsc::channel();
        let (parsed_tx, parsed_rx) = mpsc::channel();
        let output = Buffer::default();
        let stalled = StalledWriter {
            entered: Some(entered_tx),
            release: release_rx,
            exited: exited_tx,
            buffer: output.clone(),
        };
        let file: Box<dyn Write + Send>;
        let console: Box<dyn Write + Send>;
        if stall_file {
            file = Box::new(stalled);
            console = Box::new(io::sink());
        } else {
            file = Box::new(io::sink());
            console = Box::new(stalled);
        }
        let mut lines = vec!["[PROBE] ordinary marker"; protocol_log::QUEUE_CAPACITY + 8];
        lines.extend([
            "[PROBE] PCM_READY",
            "[PROBE] PCM_READY",
            "[PROBE] PCM_CLOCK start_qpc=7 frequency=10000000 prebuffer_frames=5984 lead_ms=300",
            "[PROBE] PCM_STALL timeout_ms=1000 password=fixture-secret",
            "[PROBE] PCM_LATE deadline_ms=1234",
            "[PROBE] ERROR code=PASSWORD_REJECTED exit=10 host=fixture phase=pairing",
            "[PROBE] AUDIO_TRANSPORT_OK",
        ]);
        let input = GatedInput {
            first: Cursor::new(b"[PROBE] BEGIN password=fixture-secret\n".to_vec()),
            entered: Some(entered_rx),
            rest: Cursor::new(lines.join("\n").into_bytes()),
            eof_release: None,
        };
        let mut reader = BackendReader::start_with_log(
            input,
            protocol_log::start(file, console, true).unwrap(),
            crate::privacy::Redactor::default(),
            Some(Arc::new(move |event| {
                if event["line"] == "[PROBE] AUDIO_TRANSPORT_OK" {
                    let _ = parsed_tx.send(());
                }
            })),
        )
        .unwrap();
        let ready = reader.ready.recv_timeout(Duration::from_secs(2));
        let parsed = parsed_rx.recv_timeout(Duration::from_secs(2));
        // 失败时先释放 writer 再回收读线程，避免回归测试本身永远卡住。
        // On failure release the writer before joining, so regressions cannot hang the test itself.
        if ready.is_err() || parsed.is_err() {
            drop(release_tx);
            reader.join();
            panic!("blocked writer delayed control processing: ready={ready:?}, parsed={parsed:?}");
        }
        reader.join();
        let began = Instant::now();
        let status = reader
            .finish_log(began + Duration::from_millis(20))
            .unwrap();
        let cleanup_elapsed = began.elapsed();
        drop(release_tx);
        exited_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(cleanup_elapsed < Duration::from_secs(1));
        assert_eq!(status["timed_out"], true);
        assert!(status["dropped_records"].as_u64().unwrap() > 0);
        assert_eq!(status["pending_records"], protocol_log::QUEUE_CAPACITY + 1);
        assert!(reader.ready.try_recv().is_err());
        assert!(reader.state.transport.load(Ordering::Relaxed));
        assert_eq!(reader.state.clock.lock().unwrap().unwrap().start_qpc, 7);
        assert!(
            reader
                .state
                .failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("PCM_STALL")
        );
        assert_eq!(
            reader
                .state
                .auth_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .exit,
            10
        );
        // 超时后只允许已开始的那条记录结束，不能继续输出积压；文本仍脱敏。
        // Only the in-flight record may finish after timeout; backlog is abandoned and text remains redacted.
        assert!(output.text().contains("BEGIN"));
        assert!(!output.text().contains("fixture-secret"));
        assert!(!output.text().contains("ordinary marker"));
    }
}

struct FailedWriter(&'static str);
impl Write for FailedWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, self.0))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn file_and_console_failures_are_log_status_without_losing_protocol_errors() {
    for fail_file in [true, false] {
        let message = if fail_file {
            "fixture disk failure"
        } else {
            "fixture console failure"
        };
        let failed = FailedWriter(message);
        let file: Box<dyn Write + Send>;
        let console: Box<dyn Write + Send>;
        if fail_file {
            file = Box::new(failed);
            console = Box::new(io::sink());
        } else {
            file = Box::new(io::sink());
            console = Box::new(failed);
        }
        let input = Cursor::new(b"[PROBE] PCM_READY\n[PROBE] EVENTS_FAILED reason=fixture\n[PROBE] ERROR code=PASSWORD_REJECTED exit=10 host=fixture phase=pairing\n[PROBE] AUDIO_TRANSPORT_OK".to_vec());
        let mut reader = BackendReader::start_with_log(
            input,
            protocol_log::start(file, console, true).unwrap(),
            crate::privacy::Redactor::default(),
            None,
        )
        .unwrap();
        reader.join();
        let status = reader
            .finish_log(Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert!(reader.ready.try_recv().is_ok());
        assert_eq!(status["timed_out"], false);
        assert!(status["error"].as_str().unwrap().contains(message));
        assert!(status["error"].as_str().unwrap().contains(if fail_file {
            "文件写入"
        } else {
            "控制台写入"
        }));
        assert!(reader.state.transport.load(Ordering::Relaxed));
        assert!(
            reader
                .state
                .failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("EVENTS_FAILED")
        );
        assert_eq!(
            reader
                .state
                .auth_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .exit,
            10
        );
    }
}

#[test]
fn cancellation_while_waiting_ready_does_not_wait_for_stalled_logging() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (exited_tx, exited_rx) = mpsc::channel();
    let (eof_tx, eof_rx) = mpsc::channel();
    let (parsed_tx, parsed_rx) = mpsc::channel();
    let log = protocol_log::start(
        StalledWriter {
            entered: Some(entered_tx),
            release: release_rx,
            exited: exited_tx,
            buffer: Buffer::default(),
        },
        io::sink(),
        true,
    )
    .unwrap();
    let input = GatedInput {
        first: Cursor::new(b"[PROBE] BEGIN\n".to_vec()),
        entered: Some(entered_rx),
        rest: Cursor::new(b"[PROBE] CONNECTING\n".to_vec()),
        eof_release: Some(eof_rx),
    };
    let mut reader = BackendReader::start_with_log(
        input,
        log,
        crate::privacy::Redactor::default(),
        Some(Arc::new(move |event| {
            if event["line"] == "[PROBE] CONNECTING" {
                let _ = parsed_tx.send(());
            }
        })),
    )
    .unwrap();
    parsed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let began = Instant::now();
    let result = reader.wait_ready_with(&AtomicBool::new(true), || Ok(false));
    let elapsed = began.elapsed();
    drop(eof_tx);
    reader.join();
    let status = reader
        .finish_log(Instant::now() + Duration::from_millis(20))
        .unwrap();
    drop(release_tx);
    exited_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(elapsed < Duration::from_secs(1));
    assert!(result.unwrap_err().to_string().contains("已取消建立流"));
    assert_eq!(status["timed_out"], true);
}

#[test]
fn detailed_log_preserves_password_hint_and_redaction_in_both_sinks() {
    let file = Buffer::default();
    let console = Buffer::default();
    let input = Cursor::new(
        b"[PROBE] PASSWORD_NEEDED host=192.0.2.10 password=fixture-secret\n[PROBE] PCM_READY"
            .to_vec(),
    );
    let mut reader = BackendReader::start_with_log(
        input,
        protocol_log::start(file.clone(), console.clone(), true).unwrap(),
        crate::privacy::Redactor::default(),
        None,
    )
    .unwrap();
    reader.join();
    let status = reader
        .finish_log(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert_eq!(status["dropped_records"], 0);
    assert_eq!(status["pending_records"], 0);
    assert!(status["error"].is_null());
    assert!(console.text().contains("设备要求 AirPlay 密码"));
    for text in [file.text(), console.text()] {
        assert!(text.contains("PASSWORD_NEEDED") && text.contains("PCM_READY"));
        assert!(!text.contains("fixture-secret") && !text.contains("192.0.2.10"));
    }
}
