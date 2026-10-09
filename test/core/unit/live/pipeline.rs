use super::super::transport::PcmWriter;
use super::*;
use std::{io, sync::Arc};

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
