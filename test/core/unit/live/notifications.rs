use super::*;
use std::{
    io::Cursor,
    sync::{atomic::AtomicBool, mpsc},
    time::Duration,
};

fn native(line: &str) -> Value {
    json!({"kind":"native","line":line,"safe_line":line,"is_fault":false})
}

#[test]
fn blocked_gui_does_not_delay_reader_readiness_faults_or_latest_device_statistics() {
    let control = Arc::new(GuiControl::new([0, 1]));
    let (seen, events) = mpsc::channel();
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let caller = thread::current().id();
    let mut worker = Worker::start(
        Arc::new(move |event| {
            assert_ne!(thread::current().id(), caller);
            if event["kind"] == "block" {
                entered.send(()).unwrap();
                // Dropping release also unblocks on failed assertions.
                let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(5));
            }
            seen.send(event).unwrap();
        }),
        control.clone(),
    )
    .unwrap();
    let emit = worker.emitter();
    emit(json!({"kind":"block"}));
    began.recv_timeout(Duration::from_secs(2)).unwrap();

    let mut lines = vec!["[PROBE] PCM_READY".to_owned()];
    for i in 0..1000 {
        for host in ["192.0.2.10", "192.0.2.11"] {
            lines.push(format!("[PROBE] PACKET_STATS host={host} sent={i}"));
        }
    }
    lines.push("[PROBE] PCM_STALL timeout_ms=1000".into());
    let mut reader = super::super::protocol::BackendReader::start(
        Cursor::new(lines.join("\n")),
        io::sink(),
        crate::privacy::Redactor::default(),
        false,
        Some(emit.clone()),
    )
    .unwrap();
    reader
        .wait_ready_with(&AtomicBool::new(false), || Ok(false))
        .unwrap();
    reader
        .join_until(Instant::now() + Duration::from_secs(2))
        .unwrap();
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
    assert!(!control.stop.load(Ordering::Acquire));
    for i in 0..1000 {
        emit(json!({"kind":"telemetry","output_frames":i}));
    }
    emit(json!({"kind":"capture_diagnostic","error":"fixture capture fault"}));
    {
        let queue = worker.sender.pending.0.lock().unwrap();
        assert_eq!(queue.snapshots.len(), 3);
        assert_eq!(queue.controls.len(), 3);
    }
    emit(json!({"kind":"report","report":{}}));
    emit(json!({"kind":"telemetry","output_frames":99999}));
    release.send(()).unwrap();
    worker
        .finish_until(Instant::now() + Duration::from_secs(2))
        .unwrap();
    let all: Vec<_> = events.try_iter().collect();
    assert_eq!(all.len(), 8);
    assert_eq!(all[1]["line"], "[PROBE] PCM_READY");
    let packets: Vec<_> = all
        .iter()
        .filter(|event| {
            event["line"]
                .as_str()
                .is_some_and(|line| line.contains("PACKET_STATS"))
        })
        .collect();
    assert_eq!(packets.len(), 2);
    assert!(
        packets
            .iter()
            .all(|event| event["line"].as_str().unwrap().ends_with("sent=999"))
    );
    assert!(
        all.iter()
            .any(|event| event["kind"] == "telemetry" && event["output_frames"] == 999)
    );
    assert!(all.iter().any(|event| {
        event["line"]
            .as_str()
            .is_some_and(|line| line.contains("PCM_STALL"))
    }));
    assert_eq!(all.last().unwrap()["kind"], "report");
}

#[test]
fn overflow_bounds_controls_logs_snapshots_and_report_has_reserved_capacity() {
    let control = Arc::new(GuiControl::new([0, 1]));
    let sender = Sender {
        pending: Arc::default(),
        control: control.clone(),
    };
    for _ in 0..LOGS + 3 {
        sender.send(native("ordinary output"));
    }
    for i in 0..CONTROLS {
        sender.send(json!({"kind":"ready","number":i}));
    }
    sender.send(json!({"kind":"fault"}));
    assert!(control.stop.load(Ordering::Acquire));
    assert!(control.shutdown.current().is_some());
    sender.send(json!({"kind":"report","report":{}}));
    let mut queue = sender.pending.0.lock().unwrap();
    assert_eq!(queue.controls.len(), CONTROLS);
    assert_eq!(queue.logs.len(), LOGS);
    let report = &queue.report.as_ref().unwrap().1["report"];
    assert_eq!(report["notification_status"]["dropped_display_logs"], 3);
    assert!(
        report["notification_status"]["error"]
            .as_str()
            .unwrap()
            .contains("STREAM_NOTIFY_FAILED")
    );
    let mut all = Vec::new();
    while let Some(event) = queue.pop() {
        all.push(event);
    }
    assert_eq!(all.last().unwrap()["kind"], "report");
    drop(queue);

    let snapshots = Sender {
        pending: Arc::default(),
        control: Arc::new(GuiControl::new([0, 1])),
    };
    for i in 0..SNAPSHOTS + 1 {
        snapshots.send(native(&format!(
            "[PROBE] PACKET_STATS host=fixture-{i} sent=1"
        )));
    }
    assert_eq!(
        snapshots.pending.0.lock().unwrap().snapshots.len(),
        SNAPSHOTS
    );
    assert!(snapshots.control.stop.load(Ordering::Acquire));
    let oversized = Sender {
        pending: Arc::default(),
        control: Arc::new(GuiControl::new([0, 1])),
    };
    oversized.send(native(&"x".repeat(EVENT_BYTES)));
    assert!(oversized.pending.0.lock().unwrap().logs.is_empty());
    assert!(oversized.control.stop.load(Ordering::Acquire));
}

#[test]
fn stalled_notification_cleanup_retains_handle_and_original_stop_budget() {
    let control = Arc::new(GuiControl::new([0, 1]));
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let mut worker = Worker::start(
        Arc::new(move |_| {
            entered.send(()).unwrap();
            let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(5));
        }),
        control.clone(),
    )
    .unwrap();
    worker.emitter()(json!({"kind":"ready"}));
    began.recv_timeout(Duration::from_secs(2)).unwrap();
    let deadline = control
        .shutdown
        .begin_until(Instant::now() + Duration::from_millis(30));
    assert!(
        worker
            .finish_until(deadline)
            .unwrap_err()
            .contains("STOP_TIMEOUT")
    );
    drop(worker);
    assert!(!control.cleanup_complete.load(Ordering::Acquire));
    assert!(control.notification_worker.lock().unwrap().is_some());
    assert_eq!(control.shutdown.begin(), deadline);
    release.send(()).unwrap();
    crate::shutdown::join_until(
        &mut control.notification_worker.lock().unwrap(),
        Instant::now() + Duration::from_secs(2),
        false,
    )
    .unwrap();
}

#[test]
fn callback_panic_is_an_explicit_failure_without_retaining_a_finished_worker() {
    let control = Arc::new(GuiControl::new([0, 1]));
    let mut worker = Worker::start(
        Arc::new(|_| panic!("fixture notification panic")),
        control.clone(),
    )
    .unwrap();
    worker.emitter()(json!({"kind":"ready"}));
    assert!(
        worker
            .finish_until(Instant::now() + Duration::from_secs(2))
            .unwrap_err()
            .contains("STREAM_NOTIFY_FAILED")
    );
    assert!(control.stop.load(Ordering::Acquire));
    drop(worker);
    assert!(control.cleanup_complete.load(Ordering::Acquire));
    assert!(control.notification_worker.lock().unwrap().is_none());
}
