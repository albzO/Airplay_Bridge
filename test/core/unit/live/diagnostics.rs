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
