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
