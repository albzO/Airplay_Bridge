use super::*;
fn ready_warmup() -> Warmup {
    let now = Instant::now();
    Warmup {
        since: Some(now - Duration::from_secs(1)),
        last: Some(now),
        packets: 3,
        faults: (0, 0, 0),
    }
}
fn test_source() -> Source {
    Source {
        endpoint: "test".into(),
        mapping: Mutex::new([0, 1]),
        stop: AtomicBool::new(false),
        done: AtomicBool::new(false),
        subscriber: Mutex::new(None),
        progress: Mutex::new(CaptureProgress::default()),
        worker: Mutex::new(None),
        error: Mutex::new(None),
        info: Mutex::new(Value::Null),
        diagnostic_enabled: AtomicBool::new(false),
        warmup: Mutex::new(ready_warmup()),
    }
}
#[test]
fn consumer_failure_discards_queued_audio_and_releases_duration_budget() {
    let source = Arc::new(test_source());
    let producer = source.clone();
    let worker = thread::spawn(move || {
        let began = Instant::now();
        loop {
            if let Some(sub) = producer.subscriber.lock().unwrap().as_mut() {
                let stats = sub.tx.stats.clone();
                sub.send_audio(&[0.25, -0.5], 48000, CaptureProgress::default())
                    .unwrap();
                sub.send_audio(&[9., 9.], 48000, CaptureProgress::default())
                    .unwrap();
                return stats;
            }
            assert!(began.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        }
    });
    let error = source
        .consume(
            &AtomicBool::new(false),
            &Mutex::new(CaptureProgress::default()),
            None,
            |_, _| Err("synthetic consumer failure".into()),
        )
        .unwrap_err();
    let stats = worker.join().unwrap();
    assert_eq!(error.to_string(), "synthetic consumer failure");
    assert_eq!(stats.pending_ms(), 0.0);
    assert!(stats.peak_ms() > 0.0);
    assert!(source.subscriber.lock().unwrap().is_none());
    assert!(source.is_running());
}

#[test]
fn invalid_or_over_budget_packets_skip_copy_and_large_packets_stay_intact() {
    let (tx, rx) = audio_queue::channel();
    let (buffers, recycle_tx) = buffers::Pool::new();
    let counts = buffers.stats.clone();
    let mut sub = Subscription {
        tx,
        buffers,
        fault: Arc::new(Mutex::new(None)),
        diagnostic_tx: None,
        diagnostic_drops: Arc::new(AtomicU64::new(0)),
    };
    for (samples, rate) in [(&[1.0][..], 48000), (&[1., 2.][..], 0)] {
        assert_eq!(
            sub.send_audio(samples, rate, CaptureProgress::default()),
            Err(QueueError::InvalidFormat)
        );
    }
    let large = vec![0.25; 20_000];
    assert_eq!(
        sub.send_audio(&large, 1, CaptureProgress::default()),
        Err(QueueError::Full)
    );
    assert_eq!(counts.snapshot()["created"], 0);
    sub.send_audio(&large, 48000, CaptureProgress::default())
        .unwrap();
    let mut block = rx.recv().unwrap();
    let Event::Audio(samples, rate, _, _) = &mut block.value;
    assert_eq!(*rate, 48000);
    assert_eq!(*samples, large);
    buffers::recycle(&recycle_tx, std::mem::take(samples));
    drop(block);
    assert_eq!(sub.tx.stats.pending_ms(), 0.0);
    sub.send_audio(&[0.75, -0.5], 48000, CaptureProgress::default())
        .unwrap();
    assert_eq!(counts.snapshot()["created"], 2);
    drop(rx);
    assert_eq!(sub.tx.stats.pending_ms(), 0.0);
    assert_eq!(
        sub.send_audio(&[1., 2.], 48000, CaptureProgress::default()),
        Err(QueueError::Disconnected)
    );
    assert_eq!(sub.tx.stats.pending_ms(), 0.0);
}

#[test]
fn recycled_subscription_preserves_variable_packets_rates_and_positions() {
    let source = Arc::new(test_source());
    let producer = source.clone();
    let worker = thread::spawn(move || {
        let began = Instant::now();
        let queue = loop {
            if let Some(sub) = producer.subscriber.lock().unwrap().as_ref() {
                break sub.tx.stats.clone();
            }
            assert!(began.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(1));
        };
        for index in 0..12 {
            let frames = [1, 480, 128, 1024][index % 4];
            let rate = [44100, 48000, 96000, 192000][index % 4];
            let samples: Vec<f32> = (0..frames)
                .flat_map(|frame| [frame as f32, -(index as f32)])
                .collect();
            let mut position = CaptureProgress::default();
            position.repaired_gap_frames = index as u64;
            producer
                .subscriber
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .send_audio(&samples, rate, position)
                .unwrap();
            // 等待当前块 RAII 释放，确保借用已结束且已完成归还；不依赖调度碰巧命中池。
            // Wait for block reservation release so reuse follows completed borrowing/return, not scheduling luck.
            let began = Instant::now();
            while queue.pending_ms() > 0.0 {
                assert!(began.elapsed() < Duration::from_secs(2));
                thread::sleep(Duration::from_millis(1));
            }
        }
    });
    let stop = AtomicBool::new(false);
    let progress = Mutex::new(CaptureProgress::default());
    let mut index = 0;
    let report = source
        .consume(&stop, &progress, None, |samples, rate| {
            let frames = [1, 480, 128, 1024][index % 4];
            assert_eq!(rate, [44100, 48000, 96000, 192000][index % 4]);
            assert_eq!(samples.len(), frames * 2);
            assert_eq!(progress.lock().unwrap().repaired_gap_frames, index as u64);
            for (frame, samples) in samples.chunks_exact(2).enumerate() {
                assert_eq!(samples[0].to_bits(), (frame as f32).to_bits());
                assert_eq!(samples[1].to_bits(), (-(index as f32)).to_bits());
            }
            index += 1;
            if index == 12 {
                stop.store(true, Ordering::Relaxed);
            }
            Ok(())
        })
        .unwrap();
    worker.join().unwrap();
    assert_eq!(report["frames"], 3 * (1 + 480 + 128 + 1024));
    assert_eq!(
        report["source_buffers"],
        json!({"created":1,"reused":11,"growths":2})
    );
    assert!(source.subscriber.lock().unwrap().is_none());
    assert!(source.is_running());
}
#[test]
fn zero_pcm_with_active_endpoint_cannot_pass_continuity_warmup() {
    let now = Instant::now();
    let mut warmup = ready_warmup();
    let mut progress = CaptureProgress::default();
    progress.loopback_suspect = true;
    warmup.observe(now, progress);
    assert!(!warmup.ready(now));
    progress.loopback_suspect = false;
    for ms in [10, 110, 210, 310, 410] {
        warmup.observe(now + Duration::from_millis(ms), progress);
    }
    assert!(!warmup.ready(now + Duration::from_millis(410)));
    warmup.observe(now + Duration::from_millis(510), progress);
    assert!(warmup.ready(now + Duration::from_millis(510)));
}
#[test]
fn readiness_rechecks_health_even_before_the_next_audio_callback() {
    let source = test_source();
    source.progress.lock().unwrap().loopback_suspect = true;
    let cancelled = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(40));
            cancelled.store(true, Ordering::Relaxed);
        });
        let error = source.wait_ready(&cancelled).unwrap_err();
        assert_eq!(error.to_string(), "启动采集已取消");
    });
}
#[test]
fn reopening_resets_readiness_and_counters_and_preserves_the_source() {
    let source = test_source();
    source.progress.lock().unwrap().discontinuities = 10;
    let mut captures = 0;
    let mut reopens = Vec::new();
    let report = source
        .capture_with_recovery(
            || {
                captures += 1;
                if captures == 1 {
                    return Err(capture::LoopbackStalled.into());
                }
                assert_eq!(source.progress.lock().unwrap().discontinuities, 0);
                assert!(!source.warmup.lock().unwrap().ready(Instant::now()));
                if captures == 2 {
                    Err(capture::LoopbackStalled.into())
                } else {
                    Ok(json!({"frames":480}))
                }
            },
            |attempt, previous| {
                reopens.push(attempt);
                if attempt == 1 {
                    assert_eq!(previous.discontinuities, 10);
                }
            },
        )
        .unwrap();
    assert_eq!(report["frames"], 480);
    assert_eq!(captures, 3);
    assert_eq!(reopens, [1, 2]);
    assert!(source.is_running());
}
#[test]
fn recovery_is_bounded_and_never_retries_unrelated_failures() {
    let source = test_source();
    let mut captures = 0;
    let mut reopens = 0;
    let error = source
        .capture_with_recovery(
            || {
                captures += 1;
                Err(capture::LoopbackStalled.into())
            },
            |_, _| reopens += 1,
        )
        .unwrap_err();
    assert!(error.is::<capture::LoopbackStalled>());
    assert_eq!(captures, 4);
    assert_eq!(reopens, 3);
    let error = source
        .capture_with_recovery(
            || Err("invalid timestamp".into()),
            |_, _| panic!("unrelated failure must not reopen"),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid timestamp");
}
#[test]
fn active_subscription_prevents_clock_reset_and_stopping_cancels_recovery() {
    let source = test_source();
    let (tx, _rx) = audio_queue::channel();
    *source.subscriber.lock().unwrap() = Some(Subscription {
        tx,
        buffers: buffers::Pool::new().0,
        fault: Arc::new(Mutex::new(None)),
        diagnostic_tx: None,
        diagnostic_drops: Arc::new(AtomicU64::new(0)),
    });
    let error = source
        .capture_with_recovery(
            || Err(capture::LoopbackStalled.into()),
            |_, _| panic!("active streams must not reopen"),
        )
        .unwrap_err();
    assert!(error.is::<capture::LoopbackStalled>());
    source.subscriber.lock().unwrap().take();
    let mut captures = 0;
    let report = source
        .capture_with_recovery(
            || {
                captures += 1;
                Err(capture::LoopbackStalled.into())
            },
            |_, _| source.stop.store(true, Ordering::Relaxed),
        )
        .unwrap();
    assert!(report.is_null());
    assert_eq!(captures, 1);
}
#[test]
fn warmup_requires_continuity_and_restarts_after_faults_or_gaps() {
    let start = Instant::now();
    let mut warmup = Warmup::default();
    let mut progress = CaptureProgress::default();
    assert!(!warmup.ready(start));
    for ms in [0, 100, 200, 300, 400, 500] {
        warmup.observe(start + Duration::from_millis(ms), progress);
    }
    assert!(warmup.ready(start + Duration::from_millis(500)));
    progress.discontinuities = 1;
    warmup.observe(start + Duration::from_millis(600), progress);
    assert!(!warmup.ready(start + Duration::from_millis(600)));
    for ms in [700, 800, 900, 1000, 1100] {
        warmup.observe(start + Duration::from_millis(ms), progress);
    }
    assert!(warmup.ready(start + Duration::from_millis(1100)));
    assert!(!warmup.ready(start + Duration::from_millis(1400)));
    warmup.observe(start + Duration::from_millis(1400), progress);
    assert!(!warmup.ready(start + Duration::from_millis(1400)));
}
#[test]
#[ignore = "reopens a real playback endpoint three times; no HomePod connection"]
fn real_playback_startup_progress() {
    let endpoint = std::env::var("AIRPLAY_LOOPBACK_ENDPOINT")
        .unwrap_or_else(|_| capture::default_endpoint("playback").unwrap());
    let input = capture::enumerate()
        .unwrap()
        .into_iter()
        .find(|i| i.id == endpoint && i.flow == "playback")
        .unwrap();
    let root = std::env::current_dir().unwrap().join("build/source-checks");
    for attempt in 1..=3 {
        let source = Source::start(
            root.clone(),
            input.id.clone(),
            [0, if input.channels.unwrap() > 1 { 1 } else { 0 }],
            Arc::new(|_| {}),
        );
        let ready = source.wait_ready(&AtomicBool::new(false));
        if ready.is_ok() {
            thread::sleep(Duration::from_secs(2));
        }
        let progress = *source.progress.lock().unwrap();
        let running = source.is_running();
        source.stop();
        println!(
            "playback startup attempt {attempt}: {}",
            serde_json::to_string(&progress).unwrap()
        );
        ready.unwrap();
        assert!(running);
        assert!(!progress.loopback_suspect);
    }
}
#[test]
#[ignore = "starts/stops/restarts real default recording endpoint capture; no HomePod"]
fn real_source_restart_waits_for_stable_capture() {
    let input = capture::enumerate()
        .unwrap()
        .into_iter()
        .find(|i| i.id == capture::default_endpoint("recording").unwrap())
        .unwrap();
    for _ in 0..2 {
        let source = Source::start(
            std::env::current_dir().unwrap(),
            input.id.clone(),
            [0, 1],
            Arc::new(|_| {}),
        );
        let cancelled = AtomicBool::new(false);
        source.wait_ready(&cancelled).unwrap();
        assert!(source.is_running());
        cancelled.store(true, Ordering::Relaxed);
        assert!(source.wait_ready(&cancelled).is_err());
        source.stop();
        assert!(!source.is_running());
        assert!(source.wait_ready(&AtomicBool::new(false)).is_err());
    }
}
#[test]
#[ignore = "reads real default recording endpoint and attaches/detaches two short consumers; no HomePod"]
fn real_source_keeps_capture_running_across_two_stream_consumers() {
    let input = capture::enumerate()
        .unwrap()
        .into_iter()
        .find(|i| i.id == capture::default_endpoint("recording").unwrap())
        .unwrap();
    let source = Source::start(
        std::env::current_dir().unwrap(),
        input.id,
        [0, 1],
        Arc::new(|_| {}),
    );
    let mut previous = 0;
    for _ in 0..2 {
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        let report = source
            .consume(
                &stop,
                &Mutex::new(CaptureProgress::default()),
                None,
                |samples, _| {
                    assert!(!samples.is_empty());
                    if started.elapsed() > Duration::from_millis(250) {
                        stop.store(true, Ordering::Relaxed);
                    }
                    Ok(())
                },
            )
            .unwrap();
        assert!(report["frames"].as_u64().unwrap() > 0);
        let position = report["capture_position"]["device_position"]
            .as_u64()
            .unwrap();
        assert!(position > previous);
        previous = position;
        assert!(source.is_running());
        thread::sleep(Duration::from_millis(100));
    }
    source.stop();
    assert!(!source.is_running());
}
#[test]
fn stream_detach_keeps_source_alive_and_next_attach_has_no_old_audio() {
    let source = Arc::new(test_source());
    for value in [0.25, 0.75] {
        let producer = source.clone();
        let worker = thread::spawn(move || {
            let started = Instant::now();
            loop {
                if let Some(sub) = producer.subscriber.lock().unwrap().as_mut() {
                    sub.send_audio(&[value, value], 48000, CaptureProgress::default())
                        .unwrap();
                    let _ = sub.send_audio(&[9., 9.], 48000, CaptureProgress::default());
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(2));
                thread::sleep(Duration::from_millis(1));
            }
        });
        let stop = AtomicBool::new(false);
        let report = source
            .consume(
                &stop,
                &Mutex::new(CaptureProgress::default()),
                None,
                |samples, _| {
                    assert_eq!(samples, [value, value]);
                    stop.store(true, Ordering::Relaxed);
                    Ok(())
                },
            )
            .unwrap();
        worker.join().unwrap();
        assert_eq!(report["frames"], 1);
        assert!(source.is_running());
        assert!(!source.stop.load(Ordering::Relaxed));
        assert!(source.subscriber.lock().unwrap().is_none());
    }
}
