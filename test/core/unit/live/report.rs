use super::*;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("airplay-report-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn blocked_report_preserves_protocol_capture_pipe_and_transport_priority_and_emits() {
    let fixture = Fixture::new("blocked");
    let path = fixture.0.join("report.json");
    fs::create_dir(&path).unwrap();
    for (protocol, capture, pipe, transport, expected) in [
        (true, true, true, false, "fixture protocol"),
        (false, true, true, false, "fixture capture"),
        (false, false, true, false, "fixture pipe"),
        (false, false, false, false, "持续流发送失败"),
        (false, false, false, true, "REPORT_WRITE_FAILED"),
    ] {
        let primary = session_result(
            protocol.then(|| "fixture protocol".into()),
            if capture {
                Err("fixture capture".into())
            } else {
                Ok(())
            },
            if pipe {
                Err("fixture pipe".into())
            } else {
                Ok(())
            },
            transport,
            Path::new("fixture.log"),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        let emit: GuiEmitter = Arc::new(move |event| collected.lock().unwrap().push(event));
        let error = finish(
            &path,
            serde_json::json!({"device":"fixture"}),
            primary,
            Some(&emit),
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["kind"], "report");
        assert_eq!(events[0]["report"]["device"], "fixture");
        let write_error = events[0]["report"]["report_write_error"].as_str().unwrap();
        assert!(write_error.contains("REPORT_WRITE_FAILED"), "{write_error}");
        assert!(write_error.contains("写入串流报告"), "{write_error}");
        assert!(write_error.contains("os error"), "{write_error}");
    }
}

#[test]
fn saved_report_matches_gui_snapshot_and_does_not_hide_the_primary_error() {
    let fixture = Fixture::new("saved");
    let path = fixture.0.join("report.json");
    for failed in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        let emit: GuiEmitter = Arc::new(move |event| collected.lock().unwrap().push(event));
        let primary = if failed {
            Err("fixture primary".into())
        } else {
            Ok(())
        };
        let result = finish(
            &path,
            serde_json::json!({"capture":null}),
            primary,
            Some(&emit),
        );
        assert_eq!(
            result.err().map(|e| e.to_string()),
            failed.then(|| "fixture primary".to_string())
        );
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved["report_write_error"].is_null());
        assert_eq!(saved, events.lock().unwrap()[0]["report"]);
    }
}
