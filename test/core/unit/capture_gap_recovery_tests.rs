use super::*;
#[test]
fn capture_ready_event_is_auto_reset_and_idle_wait_is_bounded() {
    let event = CaptureEvent::new().unwrap();
    assert!(!event.wait(0).unwrap());
    unsafe {
        windows::Win32::System::Threading::SetEvent(event.0).unwrap();
    }
    assert!(event.wait(20).unwrap());
    assert!(!event.wait(0).unwrap());
}
#[test]
fn raw_summary_separates_channels_and_nonfinite_float_samples() {
    let format = Format {
        rate: 48000,
        channels: 2,
        bits: 32,
        valid_bits: 32,
        block_align: 8,
        channel_mask: 3,
        encoding: "float32".into(),
    };
    let bytes: Vec<u8> = [0.0f32, 0.75, -0.25, f32::NAN]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let summary = raw_packet_summary(&bytes, &format);
    assert_eq!(summary["channel_peaks"], serde_json::json!([0.25, 0.75]));
    assert_eq!(summary["nonfinite_samples"], 1);
    assert_eq!(summary["bytes"], 16);
    // Negative float zero has nonzero raw bytes but no audio signal.
    let negative_zero: Vec<u8> = [-0.0f32, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let summary = raw_packet_summary(&negative_zero, &format);
    assert_eq!(summary["channel_peaks"], serde_json::json!([0.0, 0.0]));
    assert_eq!(summary["nonzero_bytes"], 1);
}
#[test]
fn short_gap_requires_consistent_clock_and_rejects_bad_timeline() {
    let previous = (0, 480, 1_000_000);
    assert_eq!(
        recoverable_gap(previous, 5760, 2_200_000, 48000, false).unwrap(),
        5280
    );
    assert_eq!(
        recoverable_gap(previous, 480, 1_100_000, 48000, false).unwrap(),
        0
    );
    assert!(recoverable_gap(previous, 5760, 1_100_000, 48000, false).is_err());
    assert!(recoverable_gap(previous, 5760, 2_200_000, 48000, true).is_err());
    assert!(recoverable_gap(previous, 0, 1_100_000, 48000, false).is_err());
    assert!(recoverable_gap(previous, 14400, 4_000_000, 48000, false).is_err());
}
#[test]
fn timestamp_regression_is_rejected_even_without_error_flag() {
    assert!(validate_packet_timestamp(Some(2_000_000), 1_844_185, false).is_err());
    assert!(validate_packet_timestamp(Some(2_000_000), 2_000_000, false).is_err());
    assert!(validate_packet_timestamp(None, 2_000_000, true).is_err());
    assert!(validate_packet_timestamp(Some(2_000_000), 2_100_000, false).is_ok());
}
