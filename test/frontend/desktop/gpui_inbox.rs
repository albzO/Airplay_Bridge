use super::*;
use serde_json::json;

#[test]
fn slow_ui_keeps_latest_snapshots_and_terminal_event() {
    let (inbox, wake) = Inbox::new();
    let sink = inbox.sink();
    sink(DesktopEvent::SourceLevel(json!({
        "endpoint": "sample", "captureEnabled": true, "warning": "fixture warning"
    })));
    for step in 0..100 {
        sink(DesktopEvent::SourceLevel(json!({
            "endpoint": "sample", "peaks": [0.0, step as f32 / 100.0]
        })));
        sink(DesktopEvent::Stream {
            session_id: 3,
            payload: json!({ "kind": "telemetry" }),
        });
    }
    for index in 0..CONTROL_CAPACITY + 1 {
        sink(DesktopEvent::Stream {
            session_id: 3,
            payload: json!({ "kind": "password_required", "host": index.to_string() }),
        });
    }
    sink(DesktopEvent::Stream {
        session_id: 3,
        payload: json!({ "kind": "finished" }),
    });

    assert_eq!(wake.len(), 1);
    let batch = inbox.take();
    let source = batch.source.unwrap();
    assert_eq!(source.peaks, Some([0.0, 0.99]));
    assert_eq!(source.capture_enabled, Some(true));
    assert_eq!(source.warning.as_deref(), Some("fixture warning"));
    assert_eq!(batch.playing, Some(3));
    assert_eq!(batch.controls.len(), CONTROL_CAPACITY);
    assert!(
        matches!(batch.controls.front(), Some((3, StreamUpdate::Password(host))) if host == "0")
    );
    assert_eq!(batch.overflow, Some(3));
    assert!(matches!(
        batch.finished,
        Some((3, StreamUpdate::Finished { .. }))
    ));
}
