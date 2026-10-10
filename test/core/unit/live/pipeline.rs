use super::super::transport::PcmWriter;
use super::*;
use std::{
    io::{self, Write},
    sync::Arc,
};

#[test]
fn speaker_swap_preserves_frames_and_reuses_the_session_buffer() {
    let mut buffer = Vec::new();
    let input = [0.25, -0.5, 0.75, -1.0];
    assert_eq!(route(&input, true, &mut buffer), [-0.5, 0.25, -1.0, 0.75]);
    let pointer = buffer.as_ptr();
    assert_eq!(route(&input, false, &mut buffer).as_ptr(), input.as_ptr());
    assert_eq!(route(&input[..2], true, &mut buffer), [-0.5, 0.25]);
    assert_eq!(buffer.as_ptr(), pointer);
    assert_eq!(route(&input, true, &mut buffer), [-0.5, 0.25, -1.0, 0.75]);
    assert_eq!(buffer.as_ptr(), pointer);
}

// 仅按显式命令运行；测量无设备、无协议时钟、无日志的真实转换/发送路径。
// Run explicitly only; measure actual conversion/delivery with no device, protocol clock or logging.
#[test]
#[ignore = "synthetic release performance baseline; writes ignored artifacts, no devices"]
fn synthetic_performance_baseline() {
    let mut results = Vec::new();
    for rate in [44100, 48000, 96000, 192000] {
        for round in 0..4 {
            let (sender, writer) = PcmWriter::start(io::sink());
            let counters = writer.counters.clone();
            let protocol = ProtocolState::default();
            let progress = Mutex::new(CaptureProgress::default());
            let mut audio = AudioPipeline::new(
                PipelineOptions {
                    gui: None,
                    stereo: false,
                    seed: 1234,
                    protocol: &protocol,
                    progress: &progress,
                    capture_diagnostics: false,
                },
                sender,
                writer.counters.clone(),
                None,
                None,
            );
            let samples: Vec<f32> = (0..rate / 100).flat_map(|_| [0.25, -0.5]).collect();
            let mut timings = Vec::new();
            let started = Instant::now();
            for _ in 0..500 {
                let began = Instant::now();
                audio.process(&samples, rate).unwrap();
                timings.push(began.elapsed().as_secs_f64() * 1_000_000.0);
            }
            audio.finish_audio().unwrap();
            audio.close_input();
            let bytes = writer.join().unwrap().unwrap();
            let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(bytes, 5 * 44100 * BYTES_PER_FRAME);
            timings.sort_by(f64::total_cmp);
            if round > 0 {
                results.push(
                    serde_json::json!({"input_rate":rate,"round":round,"audio_seconds":5,
                    "wall_ms":wall_ms,"pcm_queue_peak_ms":counters.queue.peak_ms(),
                    "pcm_buffers_created":counters.buffers_created.load(Ordering::Relaxed),
                    "pcm_buffers_reused":counters.buffers_reused.load(Ordering::Relaxed),
                    "process_us":{"p50":timings[250],"p95":timings[475],
                    "p99":timings[495],"max":timings[499]},"output_bytes":bytes}),
                );
            }
        }
    }
    let name = std::env::var("AIRPLAY_BASELINE_NAME").unwrap_or("performance-baseline.json".into());
    let filename = std::path::Path::new(&name);
    assert!(
        matches!(
            filename.components().next(),
            Some(std::path::Component::Normal(_))
        ) && filename.components().count() == 1
            && filename.extension().is_some_and(|e| e == "json"),
        "AIRPLAY_BASELINE_NAME must be a JSON filename, not a path"
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test/.artifacts/core")
        .join(filename);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec_pretty(&serde_json::json!({
        "profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "fixture":"5 seconds stereo, 10 ms packets, one warmup plus three rounds per rate; io::sink; no pacing/clock/logs",
        "results":results})).unwrap()).unwrap();
    println!("Synthetic baseline: {}", path.display());
}

#[test]
fn variable_packets_keep_stereo_duration_and_drain_the_filter_before_eof() {
    struct MemoryPipe(Arc<Mutex<Vec<u8>>>);
    impl Write for MemoryPipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (sender, writer) = PcmWriter::start(MemoryPipe(bytes.clone()));
    let protocol = ProtocolState::default();
    let progress = Mutex::new(CaptureProgress::default());
    let mut audio = AudioPipeline::new(
        PipelineOptions {
            gui: None,
            stereo: false,
            seed: 1234,
            protocol: &protocol,
            progress: &progress,
            capture_diagnostics: false,
        },
        sender,
        writer.counters.clone(),
        None,
        None,
    );
    for frames in [17, 503, 241, 1639] {
        let samples: Vec<f32> = (0..frames).flat_map(|_| [0.25, -0.5]).collect();
        audio.process(&samples, 48000).unwrap();
    }
    audio.finish_audio().unwrap();
    audio.close_input();
    assert_eq!(audio.captured_frames(), 2400);
    assert_eq!(
        serde_json::to_value(audio.conversion_stats()).unwrap()["output_frames"],
        2205
    );
    assert_eq!(writer.join().unwrap().unwrap(), 2205 * BYTES_PER_FRAME);
    let bytes = bytes.lock().unwrap();
    let pcm: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    for frame in pcm.chunks_exact(2).skip(500).take(500) {
        assert!((8188..=8196).contains(&frame[0]));
        assert!((-16388..=-16380).contains(&frame[1]));
    }
}

#[test]
fn failed_drift_file_does_not_interrupt_audio_conversion_or_delivery() {
    use super::super::protocol::Clock;
    use windows::Win32::System::Performance::QueryPerformanceFrequency;
    let directory =
        std::env::temp_dir().join(format!("airplay-drift-failure-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    // 目录不能作为日志文件打开；失败发生在日志线程，不应通过 process 传播。
    // A directory cannot be opened as a log file; worker failure must not propagate through process.
    let drift = DetailLog::start_drift(Some(&directory), false).unwrap();
    let (sender, writer) = PcmWriter::start(io::sink());
    let protocol = ProtocolState::default();
    let progress = Mutex::new(CaptureProgress::default());
    let mut now = 0i64;
    let mut frequency = 0i64;
    unsafe {
        QueryPerformanceCounter(&mut now).unwrap();
        QueryPerformanceFrequency(&mut frequency).unwrap();
    }
    *protocol.clock.lock().unwrap() = Some(Clock {
        start_qpc: (now - frequency * 11) as u64,
        frequency: frequency as u64,
        prebuffer_frames: 5984,
        lead_ms: 300,
    });
    let mut audio = AudioPipeline::new(
        PipelineOptions {
            gui: None,
            stereo: false,
            seed: 1234,
            protocol: &protocol,
            progress: &progress,
            capture_diagnostics: false,
        },
        sender,
        writer.counters.clone(),
        None,
        Some(drift),
    );
    audio.process(&vec![0.25; 960], 48000).unwrap();
    audio.process(&vec![0.25; 960], 48000).unwrap();
    audio.finish_audio().unwrap();
    audio.close_input();
    assert!(writer.join().unwrap().unwrap() > 0);
    let status = audio
        .finish_drift(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(!status["error"].is_null());
    assert_eq!(status["timed_out"], false);
    std::fs::remove_dir(directory).unwrap();
}
