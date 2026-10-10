use super::*;
use std::time::Duration;
fn info(index: u64, device_position: u64, qpc: u64) -> PacketInfo {
    PacketInfo {
        index,
        device_position,
        qpc,
        available: 480,
        flags: 0,
    }
}
fn data(packet: PacketInfo, received: Instant) -> PacketTrace {
    PacketTrace {
        packet,
        received,
        elapsed_ms: 10.0,
        used: 470,
        skipped: 10,
        prefix_silence: 5,
        frames_before: 100,
        mapping: [1, 0],
        raw: json!({"all_channels": true}),
    }
}
#[test]
fn packet_schema_deltas_endpoint_and_sink_failures_survive_extraction() {
    let mut entries = Vec::new();
    let mut callback = |entry| entries.push(entry);
    let enabled = AtomicBool::new(true);
    let mut trace = Trace::new(Some(&mut callback), Some(&enabled));
    let now = Instant::now();
    let metrics = PacketMetrics::measure(&[0.25, -0.5], true);
    let first = trace.packet(data(info(0, 100, 10_000), now), 48000, &metrics);
    assert!(first["read_interval_ms"].is_null());
    assert_eq!(first["mapping"], json!([1, 0]));
    assert_eq!(first["raw_packet"], json!({"all_channels":true}));
    assert_eq!(first["sample_fingerprint"].as_str().unwrap().len(), 16);
    enabled.store(false, Ordering::Relaxed);
    assert!(!trace.active());
    // 生命周期事件不受逐包诊断开关影响。
    // Lifecycle entries remain independent of the packet tracing switch.
    trace.fallback(20.0, 480);
    enabled.store(true, Ordering::Relaxed);
    let mut entry = Some(trace.packet(
        data(info(1, 580, 210_000), now + Duration::from_millis(20)),
        48000,
        &metrics,
    ));
    let p = CaptureProgress {
        windows_endpoint_peak: Some(0.5),
        windows_endpoint_peak_age_ms: Some(2.0),
        windows_endpoint_muted: Some(false),
        windows_endpoint_volume: Some(0.7),
        loopback_suspect: true,
        ..Default::default()
    };
    Trace::endpoint(&mut entry, Some(p));
    let value = entry.as_ref().unwrap();
    assert_eq!(value["read_interval_ms"], 20.0);
    assert_eq!(value["device_delta_frames"], 480);
    assert_eq!(value["timestamp_delta_ms"], 20.0);
    assert_eq!(value["windows_endpoint_muted"], false);
    assert_eq!(value["loopback_suspect"], true);
    trace.complete(entry, now, now, &Err("sink failed".into()));
    drop(trace);
    assert_eq!(entries[0]["kind"], "capture_event_fallback");
    assert_eq!(entries[1]["error"], "sink failed");
    assert!(entries[1]["sink_ms"].as_f64().unwrap() >= 0.0);
    assert!(entries[1]["processing_ms"].as_f64().unwrap() >= 0.0);
}
#[test]
fn lifecycle_json_and_final_report_preserve_fields_without_audio_hardware() {
    let mut entries = Vec::new();
    let mut callback = |entry| entries.push(entry);
    let enabled = AtomicBool::new(false);
    let mut trace = Trace::new(Some(&mut callback), Some(&enabled));
    let packet = PacketInfo {
        flags: 3,
        ..info(4, 200, 300)
    };
    trace.flags(packet, 2.0);
    trace.gap(packet, 480, 48000);
    trace.fault(packet, Some(400), &"timestamp reversed");
    trace.silence(3.0, 100, 10, 40.0, &Err("sink failed".into()));
    drop(trace);
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["packet_index"], 4);
    assert_eq!(entries[1]["gap_ms"], 10.0);
    assert_eq!(entries[2]["previous_packet_qpc_100ns"], 400);
    assert_eq!(entries[3]["error"], "sink failed");
    let input: Input =
        serde_json::from_value(json!({"id":"fixture","name":"fixture","description":"synthetic"}))
            .unwrap();
    let format: Format = serde_json::from_value(json!({"rate":48000,"channels":2,"bits":32,"block_align":8,"encoding":"float","channel_mask":3})).unwrap();
    let value = report(
        &Counters {
            frames: 15,
            packets: 1,
            silent_frames: 5,
            ..Default::default()
        },
        &input,
        &format,
        true,
        true,
    );
    assert_eq!(value.as_object().unwrap().len(), 13);
    assert_eq!(value["frames"], 15);
    assert_eq!(value["intentional_silent_frames"], 5);
    assert_eq!(value["input"]["id"], "fixture");
    assert_eq!(value["stopped_by_user"], true);
    assert!(!Trace::new(None, None).active());
}
