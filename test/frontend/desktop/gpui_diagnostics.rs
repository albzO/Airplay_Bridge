use super::*;
use serde_json::json;

fn push(batch: &mut Batch, id: u64, payload: Value, detailed: bool) {
    for update in decode(&payload, detailed) {
        batch.push(id, update);
    }
}

#[test]
fn diagnostic_bursts_keep_safe_text_bounded_and_preserve_the_current_report() {
    let mut diagnostics = Diagnostics::default();
    diagnostics.set_mode(DataMode::Portable);
    let mut batch = Batch::default();
    push(
        &mut batch,
        8,
        json!({ "kind": "log_path", "path": "private/session.log", "pipeline_path": "private/pipeline.jsonl" }),
        true,
    );
    push(
        &mut batch,
        8,
        json!({ "kind": "report", "report": { "capture": { "discontinuities": 3 } } }),
        true,
    );
    push(
        &mut batch,
        7,
        json!({ "kind": "report", "report": { "capture": { "discontinuities": 99 } } }),
        true,
    );
    for index in 0..LOG_LIMIT + 10 {
        push(
            &mut batch,
            8,
            json!({ "kind": "native", "line": "fixture raw", "safe_line": format!("safe-{index}") }),
            true,
        );
    }
    for index in 0..SUMMARY_LIMIT + 5 {
        push(
            &mut batch,
            8,
            json!({ "kind": "capture_diagnostic", "capture_frames": index }),
            true,
        );
    }
    push(
        &mut batch,
        8,
        json!({ "kind": "diagnostic_end", "status": { "note": "😀".repeat(TEXT_LIMIT) } }),
        true,
    );
    diagnostics.accept(batch, 8, &[]);

    assert_eq!(diagnostics.logs.len(), LOG_LIMIT);
    assert_eq!(
        diagnostics.logs.front().map(String::as_str),
        Some("safe-10")
    );
    assert_eq!(diagnostics.summaries.len(), SUMMARY_LIMIT);
    let end = diagnostics.summaries.back().unwrap();
    assert!(end.starts_with("诊断结束"));
    assert!(end.encode_utf16().count() <= TEXT_LIMIT);
    assert_eq!(
        diagnostics.pipeline_path.as_deref(),
        Some("[程序目录]/data/logs/pipeline.jsonl")
    );
    assert_eq!(
        diagnostics.report.as_ref().unwrap().discontinuities,
        Some(3)
    );
    diagnostics.summaries.clear();
    assert_eq!(diagnostics.logs.len(), LOG_LIMIT);
    assert_eq!(
        diagnostics.report.as_ref().unwrap().discontinuities,
        Some(3)
    );
}

#[test]
fn fault_mode_uses_safe_lines_and_limits_the_full_report() {
    let mut batch = Batch::default();
    push(
        &mut batch,
        3,
        json!({ "kind": "native", "line": "raw", "safe_line": "ordinary" }),
        false,
    );
    push(
        &mut batch,
        3,
        json!({ "kind": "native", "line": "raw", "safe_line": "fault", "is_fault": true }),
        false,
    );
    push(
        &mut batch,
        3,
        json!({ "kind": "native", "line": "raw", "is_fault": true }),
        false,
    );
    push(
        &mut batch,
        3,
        json!({ "kind": "report", "report": { "extra": "中".repeat(REPORT_LIMIT / 2) } }),
        false,
    );
    let mut diagnostics = Diagnostics::default();
    diagnostics.accept(batch, 3, &[]);
    assert_eq!(diagnostics.logs, ["fault"]);
    assert!(diagnostics.report.unwrap().text.contains("1 MiB"));
}

#[test]
fn device_totals_handle_partial_samples_resets_and_reconnects() {
    let mut diagnostics = Diagnostics::default();
    let mut batch = Batch::default();
    for fields in [
        "sent=10 rtx_resent=4",
        "sent=10",
        "sent=NaN rtx_resent=-1",
        "sent=12 rtx_resent=5",
        "sent=1",
        "sent=3",
    ] {
        push(
            &mut batch,
            2,
            json!({ "kind": "native", "line": format!("[PROBE] PACKET_STATS host=192.0.2.10 {fields}") }),
            false,
        );
    }
    diagnostics.accept(batch, 2, &[]);
    assert_eq!(
        diagnostics.devices[0].totals,
        [Some(14), None, None, None, Some(5), None]
    );
    diagnostics.begin(&Settings::default());
    let mut batch = Batch::default();
    push(
        &mut batch,
        3,
        json!({ "kind": "native", "line": "[PROBE] PACKET_STATS host=192.0.2.10 sent=1" }),
        false,
    );
    diagnostics.accept(batch, 3, &[]);
    assert_eq!(diagnostics.devices[0].totals[0], Some(15));
}
