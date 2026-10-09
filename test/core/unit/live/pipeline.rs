use super::super::transport::PcmWriter;
use super::*;
use std::{
    io::{self, Write},
    sync::Arc,
};

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
