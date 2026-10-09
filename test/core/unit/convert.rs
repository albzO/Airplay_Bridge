use super::*;
fn convert(samples: &[f32], chunk: usize) -> Vec<i16> {
    let mut converter = Converter::new(48000, 1234).unwrap();
    let mut result = Vec::new();
    let mut sink = |samples: &[i16]| {
        result.extend_from_slice(samples);
        Ok(())
    };
    for input in samples.chunks(chunk * 2) {
        converter.push(input, &mut sink).unwrap();
    }
    converter.finish(&mut sink).unwrap();
    result
}
#[test]
fn packet_boundaries_do_not_change_audio_and_channels_stay_separate() {
    let input: Vec<f32> = (0..48000)
        .flat_map(|i| {
            [
                0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin(),
                0.0,
            ]
        })
        .collect();
    let output = convert(&input, 480);
    assert_eq!(output, convert(&input, 137));
    assert_eq!(output.len(), 44100 * 2);
    let left: Vec<_> = output
        .chunks_exact(2)
        .map(|frame| frame[0] as f64 / 32768.0)
        .collect();
    let rms = (left[500..43000].iter().map(|s| s * s).sum::<f64>() / 42500.0).sqrt();
    assert!((rms - 0.5 / 2f64.sqrt()).abs() < 0.001);
    assert!(output.chunks_exact(2).all(|frame| frame[1].abs() <= 1));
}
#[test]
fn filter_rejects_out_of_band_input() {
    let input: Vec<f32> = (0..48000)
        .flat_map(|i| {
            let x = 0.5 * (2.0 * std::f32::consts::PI * 23000.0 * i as f32 / 48000.0).sin();
            [x, x]
        })
        .collect();
    let output = convert(&input, 480);
    let rms = (output[1000..86000]
        .iter()
        .map(|s| (*s as f64 / 32768.0).powi(2))
        .sum::<f64>()
        / 85000.0)
        .sqrt();
    assert!(rms < 0.001, "alias RMS {rms}");
}
#[test]
fn quantization_saturates_and_dither_has_no_mean_bias() {
    let mut q = Quantizer { state: 1 };
    let mut stats = Stats::default();
    assert_eq!(q.sample(2.0, &mut stats), 32767);
    assert_eq!(q.sample(-2.0, &mut stats), -32768);
    assert_eq!(q.sample(f32::NAN, &mut stats), 0);
    let values: Vec<_> = (0..100000).map(|_| q.sample(0.0, &mut stats)).collect();
    assert!(values.iter().all(|s| s.abs() <= 1));
    assert!(values.iter().any(|s| *s == 1) && values.iter().any(|s| *s == -1));
    assert!(
        (values.iter().map(|s| *s as i64).sum::<i64>() as f64 / values.len() as f64).abs() < 0.01
    );
}
#[test]
fn adaptive_ratio_flush_preserves_adjusted_duration_and_signal() {
    for ppm in [-800.0, 800.0] {
        let mut converter = Converter::new(48000, 1234).unwrap();
        converter.set_correction_ppm(ppm).unwrap();
        let mut output = Vec::new();
        let mut sink = |pcm: &[i16]| {
            output.extend_from_slice(pcm);
            Ok(())
        };
        for _ in 0..1001 {
            converter.push(&vec![0.25; 960], &mut sink).unwrap();
        }
        converter.push(&vec![0.25; 274], &mut sink).unwrap();
        converter.finish(&mut sink).unwrap();
        let expected =
            ((480480.0 + 137.0) * 44100.0 / 48000.0 * (1.0 + ppm / 1e6)).round() as usize;
        assert!((output.len() / 2).abs_diff(expected) <= 1);
        assert!(
            output[1000..output.len() - 1000]
                .iter()
                .all(|s| (*s as i32 - 8192).abs() <= 2)
        );
    }
}
#[test]
fn fractional_tail_has_exact_duration() {
    for frames in [1, 137, 479, 480, 481, 1001] {
        let output = convert(&vec![0.0; frames * 2], 137);
        assert_eq!(output.len() / 2, (frames * 44100 + 24000) / 48000);
    }
}
