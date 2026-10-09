use super::*;

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
    );
    reader.join();
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
