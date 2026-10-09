use super::*;
fn playing() -> PlaybackReading {
    PlaybackReading {
        peak: Some(0.39),
        muted: Some(false),
        volume: Some(0.5),
    }
}
#[test]
fn active_output_with_zero_raw_pcm_suspends_readiness_before_recovery() {
    let start = Instant::now();
    let mut health = LoopbackHealth::default();
    assert!(health.observe(start, playing(), false));
    assert!(!health.stalled(start + Duration::from_millis(499)));
    assert!(health.stalled(start + Duration::from_millis(500)));
    assert!(!health.observe(start + Duration::from_millis(510), playing(), true));
    assert!(!health.stalled(start + Duration::from_secs(1)));
}
#[test]
fn silence_mute_zero_volume_and_missing_controls_never_request_reopening() {
    let start = Instant::now();
    for reading in [
        PlaybackReading {
            peak: Some(0.0),
            ..playing()
        },
        PlaybackReading {
            muted: Some(true),
            ..playing()
        },
        PlaybackReading {
            volume: Some(0.0),
            ..playing()
        },
        PlaybackReading {
            peak: None,
            ..playing()
        },
        PlaybackReading {
            muted: None,
            ..playing()
        },
        PlaybackReading {
            volume: None,
            ..playing()
        },
    ] {
        let mut health = LoopbackHealth::default();
        assert!(!health.observe(start, reading, false));
        assert!(!health.observe(start + Duration::from_secs(2), reading, false));
        assert!(!health.stalled(start + Duration::from_secs(2)));
    }
}
#[test]
fn a_short_mismatch_cannot_accumulate_across_valid_packets_or_quiet_periods() {
    let start = Instant::now();
    let mut health = LoopbackHealth::default();
    health.observe(start, playing(), false);
    health.observe(
        start + Duration::from_millis(400),
        PlaybackReading {
            peak: Some(0.0),
            ..playing()
        },
        false,
    );
    health.observe(start + Duration::from_millis(500), playing(), false);
    assert!(!health.stalled(start + Duration::from_millis(900)));
    assert!(health.stalled(start + Duration::from_millis(1000)));
}

#[test]
fn established_capture_survives_pause_with_an_active_or_stale_meter_and_resumes() {
    let start = Instant::now();
    let mut health = LoopbackHealth::default();
    for ms in (0..=500).step_by(10) {
        assert!(!health.observe(start + Duration::from_millis(ms), playing(), true));
    }
    // 播放器暂停后电平仍非零也不能重建；恢复播放、再次暂停不重新开启启动检查。
    // A nonzero meter during pause must not reopen capture; resume and another pause never rearm startup.
    for ms in (510..=10_000).step_by(10) {
        assert!(!health.observe(start + Duration::from_millis(ms), playing(), false));
        assert!(!health.stalled(start + Duration::from_millis(ms)));
    }
    assert!(!health.observe(start + Duration::from_secs(11), playing(), true));
    assert!(!health.observe(start + Duration::from_secs(12), playing(), false));
    assert!(!health.stalled(start + Duration::from_secs(13)));
}

#[test]
fn quiet_startup_is_valid_and_does_not_rearm_on_later_meter_changes() {
    let start = Instant::now();
    let mut health = LoopbackHealth::default();
    let quiet = PlaybackReading {
        peak: Some(0.0),
        ..playing()
    };
    for ms in (0..=500).step_by(10) {
        assert!(!health.observe(start + Duration::from_millis(ms), quiet, false));
    }
    assert!(!health.observe(start + Duration::from_secs(1), playing(), false));
    assert!(!health.stalled(start + Duration::from_secs(2)));
}

#[test]
fn a_brief_initial_signal_does_not_hide_a_startup_loopback_failure() {
    let start = Instant::now();
    let mut health = LoopbackHealth::default();
    for ms in (0..=400).step_by(10) {
        assert!(!health.observe(start + Duration::from_millis(ms), playing(), true));
    }
    assert!(health.observe(start + Duration::from_millis(410), playing(), false));
    assert!(health.observe(start + Duration::from_millis(910), playing(), false));
    assert!(health.stalled(start + Duration::from_millis(910)));
}
