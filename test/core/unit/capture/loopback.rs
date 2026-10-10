use super::*;
#[test]
#[ignore = "opens a real playback endpoint for three seconds; no HomePod connection"]
fn real_playback_capture_keeps_running() {
    let devices = enumerate().unwrap();
    let selected = std::env::var("AIRPLAY_LOOPBACK_SOURCE")
        .unwrap_or_else(|_| default_endpoint("playback").unwrap());
    let source = devices
        .iter()
        .find(|d| d.flow == "playback" && (d.id == selected || d.name == selected))
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.flow == "playback" && d.channels == Some(2))
        })
        .expect("no stereo playback source");
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut received = 0u64;
    let _com = Com::open().unwrap();
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap() };
    let wide: Vec<u16> = source.id.encode_utf16().chain(Some(0)).collect();
    let device = unsafe {
        enumerator
            .GetDevice(windows::core::PCWSTR(wide.as_ptr()))
            .unwrap()
    };
    let meter: windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation =
        unsafe { device.Activate(CLSCTX_ALL, None).unwrap() };
    let mut captured_peak = 0f32;
    let mut endpoint_peak = 0f32;
    let report = live_selected(
        Path::new("."),
        3,
        &stop,
        Some(&source.id),
        None,
        |samples, rate| {
            assert_eq!(rate, source.rate.unwrap());
            assert!(samples.iter().all(|v| v.is_finite()));
            received += samples.len() as u64 / 2;
            for sample in samples {
                captured_peak = captured_peak.max(sample.abs());
            }
            endpoint_peak = endpoint_peak.max(unsafe { meter.GetPeakValue().unwrap_or(0.0) });
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(received, source.rate.unwrap() as u64 * 3);
    assert_eq!(report["loopback"], true);
    println!(
        "PASS playback loopback: {received} frames; {} intentional silent frames; {} device packets",
        report["intentional_silent_frames"], report["packets"]
    );
    println!(
        "{}: capture peak={captured_peak:.6}, Windows endpoint peak={endpoint_peak:.6}, non-silent frames={}",
        source.name, report["signal_frames"]
    );
}
