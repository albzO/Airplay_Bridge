use super::*;

fn format(bits: u16, channels: u16) -> Format {
    Format {
        rate: 48000,
        channels,
        bits,
        valid_bits: bits,
        block_align: channels * (bits / 8),
        encoding: "pcm".into(),
        channel_mask: 0,
    }
}

#[test]
fn pcm_storage_widths_decode_signed_extremes_and_unsigned_silence() {
    // Fixed little-endian bytes, including sign extension across the 24-bit boundary.
    let cases: &[(u16, &[u8], &[f64])] = &[
        (8, &[0, 128, 192, 255], &[-1.0, 0.0, 0.5, 127.0 / 128.0]),
        (
            16,
            &[0, 128, 0, 0, 0, 64, 255, 127],
            &[-1.0, 0.0, 0.5, 32767.0 / 32768.0],
        ),
        (
            24,
            &[0, 0, 128, 0, 0, 0, 0, 0, 64, 255, 255, 127, 255, 255, 255],
            &[-1.0, 0.0, 0.5, 8388607.0 / 8388608.0, -1.0 / 8388608.0],
        ),
        (
            32,
            &[0, 0, 0, 128, 0, 0, 0, 0, 0, 0, 0, 64, 255, 255, 255, 127],
            &[-1.0, 0.0, 0.5, 2147483647.0 / 2147483648.0],
        ),
    ];
    for &(bits, bytes, expected) in cases {
        let format = format(bits, 1);
        let decoded: Vec<_> = bytes
            .chunks_exact((bits / 8) as usize)
            .map(|bytes| sample(bytes, &format))
            .collect();
        assert_eq!(decoded, expected, "{bits}-bit PCM");
        let mut stereo = Vec::new();
        decode_stereo(bytes, &format, 0..expected.len(), None, &mut stereo).unwrap();
        let expected: Vec<_> = expected
            .iter()
            .flat_map(|value| [*value as f32; 2])
            .collect();
        assert_eq!(stereo, expected, "{bits}-bit stereo output");
    }
}

#[test]
fn pcm_valid_bits_remain_left_aligned_in_storage_container() {
    let mut format = format(32, 1);
    format.valid_bits = 24;
    let bytes = [0, 0, 0, 128, 0, 0, 0, 64, 0, 255, 255, 127];
    let mut output = Vec::new();
    decode_stereo(&bytes, &format, 0..3, None, &mut output).unwrap();
    assert_eq!(
        output,
        [
            -1.0,
            -1.0,
            0.5,
            0.5,
            8388607.0 / 8388608.0,
            8388607.0 / 8388608.0
        ]
    );
}

#[test]
fn defaults_duplicate_mono_and_preserve_stereo_order() {
    let bytes = [0, 64, 0, 192];
    let mut output = Vec::new();
    decode_stereo(&bytes, &format(16, 1), 0..2, None, &mut output).unwrap();
    assert_eq!(output, [0.5, 0.5, -0.5, -0.5]);
    decode_stereo(&bytes, &format(16, 2), 0..1, None, &mut output).unwrap();
    assert_eq!(output, [0.5, -0.5]);
}

#[test]
fn multichannel_mapping_can_reverse_or_duplicate_without_changing_raw_summary() {
    let format = format(16, 4);
    let bytes = [0, 32, 0, 64, 0, 192, 0, 128];
    let mut output = Vec::new();
    decode_stereo(&bytes, &format, 0..1, None, &mut output).unwrap();
    assert_eq!(output, [0.25, 0.5]);
    decode_stereo(&bytes, &format, 0..1, Some([3, 1]), &mut output).unwrap();
    assert_eq!(output, [-1.0, 0.5]);
    decode_stereo(&bytes, &format, 0..1, Some([2, 2]), &mut output).unwrap();
    assert_eq!(output, [-0.5, -0.5]);
    assert_eq!(
        raw_packet_summary(&bytes, &format)["channel_peaks"],
        serde_json::json!([0.25, 0.5, 0.5, 1.0])
    );
}

#[test]
fn frame_window_skips_overlap_trims_tail_and_reuses_output() {
    let format = format(16, 2);
    let bytes = [0, 128, 0, 128, 0, 64, 0, 192, 0, 32, 0, 0];
    let mut output = Vec::with_capacity(32);
    let pointer = output.as_ptr();
    let capacity = output.capacity();
    decode_stereo(&bytes, &format, 1..2, None, &mut output).unwrap();
    assert_eq!(output, [0.5, -0.5]);
    decode_stereo(&bytes, &format, 2..3, Some([1, 0]), &mut output).unwrap();
    assert_eq!(output, [0.0, 0.25]);
    decode_stereo(&bytes, &format, 3..3, None, &mut output).unwrap();
    assert!(output.is_empty());
    assert_eq!(output.as_ptr(), pointer);
    assert_eq!(output.capacity(), capacity);
    // Diagnostic peaks include the skipped frame, rather than just the decoded window.
    assert_eq!(
        raw_packet_summary(&bytes, &format)["channel_peaks"],
        serde_json::json!([1.0, 1.0])
    );
}

#[test]
fn malformed_packets_windows_and_mappings_return_errors_without_partial_output() {
    let format = format(16, 2);
    let bytes = [0, 64, 0, 192];
    let mut output = vec![0.125, -0.125];
    for (packet, frames, mapping) in [
        (&bytes[..3], 0..0, None),
        (&bytes[..], 0..2, None),
        (&bytes[..], Range { start: 1, end: 0 }, None),
        (&bytes[..], usize::MAX..usize::MAX, None),
        (&bytes[..], 0..1, Some([0, 2])),
        (&bytes[..], 0..1, Some([usize::MAX, 0])),
    ] {
        assert!(decode_stereo(packet, &format, frames, mapping, &mut output).is_err());
        assert_eq!(output, [0.125, -0.125]);
    }
}

#[test]
fn float_values_are_not_clamped_or_sanitized_during_capture_decoding() {
    let mut format = format(32, 2);
    format.encoding = "float32".into();
    let bytes: Vec<_> = [
        1.5f32,
        -2.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        -0.0,
    ]
    .into_iter()
    .flat_map(f32::to_le_bytes)
    .collect();
    let mut output = Vec::new();
    decode_stereo(&bytes, &format, 0..3, None, &mut output).unwrap();
    assert_eq!(&output[..4], &[1.5, -2.0, f32::INFINITY, f32::NEG_INFINITY]);
    assert!(output[4].is_nan());
    assert_eq!(output[5].to_bits(), (-0.0f32).to_bits());
}

#[test]
fn raw_summary_separates_channels_and_nonfinite_float_samples() {
    let mut format = format(32, 2);
    format.encoding = "float32".into();
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
