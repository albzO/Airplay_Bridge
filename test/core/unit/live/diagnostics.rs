use super::*;

#[test]
fn detailed_trace_flushes_samples_and_terminal_fault() {
    let path = std::env::temp_dir().join(format!("airplay-detail-{}.jsonl", std::process::id()));
    let trace = DetailLog::start(&path).unwrap();
    trace.record(serde_json::json!({"kind":"sample","pending_pcm_ms":150.0}));
    trace.record(serde_json::json!({"kind":"fault","error":"PCM 队列已满"}));
    let status = trace.finish();
    assert_eq!(status["dropped_records"], 0);
    assert!(status["error"].is_null());
    let contents = fs::read_to_string(&path).unwrap();
    let entries: Vec<serde_json::Value> = contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1]["kind"], "fault");
    fs::remove_file(path).unwrap();
}

#[test]
fn diagnostic_file_limit_preserves_valid_records_and_reports_truncation() {
    let path = std::env::temp_dir().join(format!(
        "airplay-diagnostic-cap-{}.jsonl",
        std::process::id()
    ));
    let log = DetailLog::start_writer(&path, 16, 64, false).unwrap();
    log.record(serde_json::json!({"kind":"start"}));
    log.record(serde_json::json!({"kind":"packet","padding":"x".repeat(100)}));
    let status = log.finish();
    assert_eq!(status["dropped_records"], 0);
    assert!(status["error"].as_str().unwrap().contains("64 字节上限"));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.len() <= 64);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(text.trim()).unwrap()["kind"],
        "start"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn rolling_diagnostic_keeps_latest_fault_and_bounds_old_segments() {
    let directory = std::env::temp_dir().join(format!("airplay-rotate-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("capture.jsonl");
    let log = DetailLog::start_writer(&path, 1024, 64, true).unwrap();
    for n in 0..20 {
        log.record(serde_json::json!({"packet":n,"payload":"1234567890"}));
    }
    log.record(serde_json::json!({"kind":"fault","error":"capture stopped"}));
    let status = log.finish();
    assert!(status["error"].is_null());
    assert_eq!(status["dropped_records"], 0);
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("capture stopped")
    );
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 4);
    for file in fs::read_dir(&directory).unwrap() {
        let file = file.unwrap();
        assert!(file.metadata().unwrap().len() <= 64);
        for line in fs::read_to_string(file.path()).unwrap().lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
        fs::remove_file(file.path()).unwrap();
    }
    fs::remove_dir(directory).unwrap();
}

#[test]
fn blocked_writer_counts_overflow_and_cleanup_has_a_deadline() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (exited_tx, exited_rx) = mpsc::channel();
    let log = DetailLog::spawn_worker(1, move |rx, pending, abandon| {
        rx.recv().unwrap();
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        pending.fetch_sub(1, Ordering::Relaxed);
        // I/O 恢复后不能继续写超时期间的积压；测试最终释放并等待模拟工作线程退出。
        // Recovered I/O must not write abandoned backlog; release and await the fixture worker on exit.
        for _ in rx {
            assert!(abandon.load(Ordering::Acquire));
            break;
        }
        exited_tx.send(()).unwrap();
        Ok(())
    })
    .unwrap();
    log.record(serde_json::json!({"packet":1}));
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    log.record(serde_json::json!({"packet":2}));
    log.record(serde_json::json!({"packet":3}));
    let began = Instant::now();
    let status = log.finish_until(began + Duration::from_millis(20));
    // 在断言之前释放 fixture，失败也不能留下故意阻塞的线程。
    // Release the fixture before assertions so a failed check cannot leave its intentional stall behind.
    release_tx.send(()).unwrap();
    exited_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(began.elapsed() < Duration::from_secs(1));
    assert_eq!(status["dropped_records"], 1);
    assert_eq!(status["pending_records"], 2);
    assert_eq!(status["timed_out"], true);
}

#[test]
fn writer_failure_is_reported_separately_from_audio() {
    let log = DetailLog::spawn_worker(16, move |rx, _, _| {
        rx.recv().unwrap();
        Err(std::io::Error::other("fixture disk failure"))
    })
    .unwrap();
    log.record(serde_json::json!({"kind":"drift"}));
    let status = log.finish();
    assert_eq!(status["timed_out"], false);
    assert_eq!(status["pending_records"], 1);
    assert!(
        status["error"]
            .as_str()
            .unwrap()
            .contains("fixture disk failure")
    );
}

#[test]
fn source_log_appends_redacts_and_rotates_before_capture_records() {
    let directory = std::env::temp_dir().join(format!("airplay-source-log-{}", std::process::id()));
    let path = directory.join("source-startup.jsonl");
    for attempt in 0..2 {
        let log = DetailLog::start_source(&path).unwrap();
        log.record(serde_json::json!({"attempt":attempt,"password":"fixture-secret"}));
        assert!(log.finish()["error"].is_null());
    }
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(!text.contains("fixture-secret"));
    fs::write(&path, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    let log = DetailLog::start_source(&path).unwrap();
    log.record(serde_json::json!({"kind":"source_requested"}));
    assert!(log.finish()["error"].is_null());
    assert_eq!(
        fs::metadata(path.with_extension("previous.jsonl"))
            .unwrap()
            .len(),
        2 * 1024 * 1024 + 1
    );
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("source_requested")
    );
    fs::remove_file(path.with_extension("previous.jsonl")).unwrap();
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();
}

#[test]
fn multiple_stalled_logs_share_one_cleanup_budget() {
    let mut logs = Vec::new();
    let mut releases = Vec::new();
    let mut exits = Vec::new();
    for _ in 0..2 {
        let (release_tx, release_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let log = DetailLog::spawn_worker(1, move |_, _, _| {
            release_rx.recv().unwrap();
            exit_tx.send(()).unwrap();
            Ok(())
        })
        .unwrap();
        logs.push(log);
        releases.push(release_tx);
        exits.push(exit_rx);
    }
    let began = Instant::now();
    let deadline = began + Duration::from_millis(20);
    let statuses: Vec<_> = logs
        .into_iter()
        .map(|log| log.finish_until(deadline))
        .collect();
    for release in releases {
        release.send(()).unwrap();
    }
    for exit in exits {
        exit.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert!(began.elapsed() < Duration::from_millis(200));
    assert!(statuses.iter().all(|status| status["timed_out"] == true));
}

#[test]
fn closed_healthy_worker_finishes_while_another_writer_is_stalled() {
    let mut healthy = DetailLog::spawn_worker(1, |rx, pending, _| {
        for _ in rx {
            pending.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    })
    .unwrap();
    healthy.record(serde_json::json!({"kind":"sample"}));
    healthy.close();
    let began = Instant::now();
    while !healthy.worker.is_finished() {
        assert!(began.elapsed() < Duration::from_secs(1));
        thread::yield_now();
    }
    // 即使共享期限已经用尽，提前关闭并已排空的健康线程也不能被误报为超时。
    // An already closed/drained healthy worker must not be reported as timed out at an expired shared deadline.
    let status = healthy.finish_until(Instant::now() - Duration::from_millis(1));
    assert_eq!(status["timed_out"], false);
    assert_eq!(status["pending_records"], 0);
    assert!(status["error"].is_null());
}
