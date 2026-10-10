use super::*;
use std::time::Duration;

#[test]
fn levels_ignore_nonfinite_samples_and_count_signal_once_per_stereo_frame() {
    let metrics = PacketMetrics::measure(
        &[
            0.25,
            -0.5,
            -0.75,
            0.125,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.4,
        ],
        false,
    );
    assert_eq!(metrics.peaks, [0.75, 0.5]);
    assert_eq!(metrics.signal_frames, 3);
    assert_eq!(metrics.fingerprint, None);
}

#[test]
fn signal_threshold_is_strict_and_does_not_hide_quiet_peaks() {
    let above = f32::from_bits(1e-6f32.to_bits() + 1);
    let metrics = PacketMetrics::measure(&[1e-6, -1e-6, above, 0.0, 0.0, -above], false);
    assert_eq!(metrics.peaks, [above, above]);
    assert_eq!(metrics.signal_frames, 2);
}

#[test]
fn fingerprint_preserves_the_existing_vector_and_can_be_disabled() {
    let input = [0.25, -0.5, 0.75, -1.0];
    let traced = PacketMetrics::measure(&input, true);
    assert_eq!(traced.fingerprint, Some(0x15e56381378e13f5));
    let untraced = PacketMetrics::measure(&input, false);
    assert_eq!(untraced.fingerprint, None);
    assert_eq!(traced.peaks, untraced.peaks);
    assert_eq!(traced.signal_frames, untraced.signal_frames);
    // Diagnostics hash raw float bits, even when both frames have no finite signal.
    let zero = PacketMetrics::measure(&[0.0, 0.0], true);
    let negative_zero = PacketMetrics::measure(&[-0.0, 0.0], true);
    assert_ne!(zero.fingerprint, negative_zero.fingerprint);
    assert_eq!(zero.peaks, negative_zero.peaks);
    let nan = PacketMetrics::measure(&[f32::from_bits(0x7fc00001), 0.0], true);
    let other_nan = PacketMetrics::measure(&[f32::from_bits(0x7fc00002), 0.0], true);
    assert_ne!(nan.fingerprint, other_nan.fingerprint);
    assert_eq!(nan.signal_frames, 0);
}

#[test]
fn empty_silent_and_incomplete_frames_keep_previous_statistics_rules() {
    let empty = PacketMetrics::measure(&[], true);
    assert_eq!(empty.peaks, [0.0; 2]);
    assert_eq!(empty.signal_frames, 0);
    assert_eq!(empty.fingerprint, Some(0xcbf29ce484222325));
    let silent = PacketMetrics::measure(&[0.0, -0.0, 0.0, 0.0], true);
    assert_eq!(silent.peaks, [0.0; 2]);
    assert_eq!(silent.signal_frames, 0);
    // Capture supplies complete frames; preserve the old chunks_exact behavior defensively.
    let incomplete = PacketMetrics::measure(&[0.0, 0.0, 1.0], true);
    assert_eq!(incomplete.peaks, [0.0; 2]);
    assert_eq!(incomplete.signal_frames, 0);
    assert_ne!(
        incomplete.fingerprint,
        PacketMetrics::measure(&[0.0, 0.0], true).fingerprint
    );
}

#[test]
fn diagnostic_clock_has_no_initial_delta_and_spans_paused_logging() {
    let mut clock = DiagnosticClock::default();
    let start = Instant::now();
    let first = clock.observe(start, 480, 1_000_000);
    assert_eq!(first.read_interval_ms, None);
    assert_eq!(first.device_delta_frames, None);
    assert_eq!(first.timestamp_delta_ms, None);
    let next = clock.observe(start + Duration::from_millis(10), 960, 1_100_000);
    assert_eq!(next.read_interval_ms, Some(10.0));
    assert_eq!(next.device_delta_frames, Some(480));
    assert_eq!(next.timestamp_delta_ms, Some(10.0));
    // Disabled logging makes no observation. Resuming compares with the last emitted record.
    let resumed = clock.observe(start + Duration::from_millis(1010), 48960, 11_100_000);
    assert_eq!(resumed.read_interval_ms, Some(1000.0));
    assert_eq!(resumed.device_delta_frames, Some(48000));
    assert_eq!(resumed.timestamp_delta_ms, Some(1000.0));
}

#[test]
fn diagnostic_differences_remain_signed_and_use_100ns_timestamp_units() {
    let mut clock = DiagnosticClock::default();
    let start = Instant::now();
    clock.observe(start, u64::MAX, 10000);
    let delta = clock.observe(start + Duration::from_micros(125), 0, 8750);
    assert_eq!(delta.read_interval_ms, Some(0.125));
    assert_eq!(delta.device_delta_frames, Some(-(u64::MAX as i128)));
    assert_eq!(delta.timestamp_delta_ms, Some(-0.125));
}
