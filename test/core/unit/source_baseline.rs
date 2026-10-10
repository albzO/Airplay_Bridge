use super::*;

// 同一生产 send_audio 路径前后对比；短批确认限制积压，不模拟实时播放。
// Compare the production send_audio path; batch acknowledgements bound backlog without real-time pacing.
#[test]
#[ignore = "synthetic Source release baseline; no devices, writes bounded ignored artifacts"]
fn synthetic_source_performance_baseline() {
    assert!(
        !cfg!(debug_assertions),
        "run this measurement with --release"
    );
    const PACKETS: usize = 5000;
    const BATCH: usize = 8;
    let mut results = Vec::new();
    for rate in [44100, 48000, 96000, 192000] {
        for round in 0..4 {
            let (tx, rx) = audio_queue::channel();
            let queue = tx.stats.clone();
            let (buffers, recycle_tx) = buffers::Pool::new();
            let buffer_stats = buffers.stats.clone();
            let subscription = Mutex::new(Subscription {
                tx,
                buffers,
                fault: Arc::new(Mutex::new(None)),
                diagnostic_tx: None,
                diagnostic_drops: Arc::new(AtomicU64::new(0)),
            });
            let samples: Vec<f32> = (0..rate / 100).flat_map(|_| [0.25, -0.5]).collect();
            let (ack, acknowledged) = mpsc::sync_channel(1);
            let mut timings = Vec::with_capacity(PACKETS);
            let started = Instant::now();
            let frames = thread::scope(|scope| {
                let consumer = scope.spawn(move || {
                    let mut frames = 0;
                    for index in 0..PACKETS {
                        let mut block = rx.recv_timeout(Duration::from_secs(2)).unwrap();
                        let Event::Audio(samples, input_rate, position, _) = &mut block.value;
                        assert_eq!(*input_rate, rate);
                        assert_eq!(position.repaired_gap_frames, index as u64);
                        assert_eq!(samples.len(), (rate / 100 * 2) as usize);
                        for frame in samples.chunks_exact(2) {
                            assert_eq!(frame[0].to_bits(), 0.25f32.to_bits());
                            assert_eq!(frame[1].to_bits(), (-0.5f32).to_bits());
                        }
                        frames += samples.len() / 2;
                        buffers::recycle(&recycle_tx, std::mem::take(samples));
                        drop(block);
                        if (index + 1) % BATCH == 0 {
                            ack.send(()).unwrap();
                        }
                    }
                    frames
                });
                for index in 0..PACKETS {
                    let mut position = CaptureProgress::default();
                    position.repaired_gap_frames = index as u64;
                    let began = Instant::now();
                    subscription
                        .lock()
                        .unwrap()
                        .send_audio(&samples, rate, position)
                        .unwrap();
                    timings.push(began.elapsed().as_secs_f64() * 1_000_000.0);
                    if (index + 1) % BATCH == 0 {
                        acknowledged.recv_timeout(Duration::from_secs(2)).unwrap();
                    }
                }
                consumer.join().unwrap()
            });
            let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(frames, PACKETS * rate as usize / 100);
            assert_eq!(queue.pending_ms(), 0.0);
            timings.sort_by(f64::total_cmp);
            let buffer_counts = buffer_stats.snapshot();
            if round > 0 {
                results.push(json!({"input_rate":rate,"round":round,"audio_seconds":50,
                    "wall_ms":wall_ms,"source_queue_peak_ms":queue.peak_ms(),
                    "source_buffers_created":buffer_counts["created"],"source_buffers_reused":buffer_counts["reused"],"source_buffer_growths":buffer_counts["growths"],
                    "process_us":{"p50":timings[PACKETS/2],"p95":timings[PACKETS*95/100],
                    "p99":timings[PACKETS*99/100],"max":timings[PACKETS-1]},"output_frames":frames}));
            }
        }
    }
    let name =
        std::env::var("AIRPLAY_SOURCE_BASELINE_NAME").unwrap_or("source-performance.json".into());
    let filename = std::path::Path::new(&name);
    assert!(
        filename.components().count() == 1
            && matches!(
                filename.components().next(),
                Some(std::path::Component::Normal(_))
            )
            && filename
                .extension()
                .is_some_and(|extension| extension == "json"),
        "use a JSON filename without directories"
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test/.artifacts/core")
        .join(filename);
    let artifact = serde_json::to_vec_pretty(&json!({"profile":"release",
        "fixture":"Source send_audio plus subscription lock; 5000 stereo 10 ms packets; batch 8 acknowledgements; one warmup plus three rounds; no WASAPI/resampler/network/logs",
        "results":results})).unwrap();
    assert!(artifact.len() < 64 * 1024);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, artifact).unwrap();
    println!("Synthetic Source baseline: {}", path.display());
}
