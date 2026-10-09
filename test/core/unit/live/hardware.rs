//! 实机检查保留原有忽略标记；测试集中在根目录 Test，保留各私有模块的测试挂接关系。
//! Hardware checks retain their ignored status; tests live under root test while retaining their private-module attachment.
use super::protocol::Clock;
use super::*;
use crate::{convert::Converter, drift::Controller};
use std::sync::{Arc, atomic::AtomicBool, mpsc};
use windows::Win32::System::Performance::QueryPerformanceCounter;
#[test]
#[ignore = "opens real 默认音频来源 for 30 seconds with a paced consumer and drift servo"]
fn real_default_source_drift_servo_with_paced_consumer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    let (tx, rx) = mpsc::sync_channel::<Vec<i16>>(64);
    let clock = Arc::new(Mutex::new(None::<Clock>));
    let consumer_clock = clock.clone();
    let consumer = thread::spawn(move || {
        let mut frames = 0u64;
        while frames < 5984 {
            frames += (rx.recv().unwrap().len() / 2) as u64;
        }
        let mut qpc = 0;
        let mut frequency = 0;
        unsafe {
            QueryPerformanceCounter(&mut qpc).unwrap();
            windows::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency).unwrap();
        }
        let origin = Clock {
            start_qpc: qpc as u64,
            frequency: frequency as u64,
            prebuffer_frames: frames,
            lead_ms: 2000,
        };
        *consumer_clock.lock().unwrap() = Some(origin);
        let began = Instant::now();
        for block in rx {
            let due = Duration::from_secs_f64(frames as f64 / 44100.0);
            if let Some(delay) = due.checked_sub(began.elapsed()) {
                thread::sleep(delay);
            }
            frames += (block.len() / 2) as u64;
        }
        frames
    });
    let mut converter = None;
    let mut controller = Controller::new();
    let mut updated = 0.0;
    let mut sink = |pcm: &[i16]| -> Result<()> {
        tx.try_send(pcm.to_vec())
            .map_err(|_| "paced queue overflow")?;
        Ok(())
    };
    let capture = capture::live(&root, 30, &AtomicBool::new(false), |samples, rate| {
        if converter.is_none() {
            converter = Some(Converter::new(rate, 1234)?);
        }
        let converter = converter.as_mut().unwrap();
        converter.push(samples, &mut sink)?;
        if let Some(origin) = *clock.lock().unwrap() {
            let elapsed = origin.elapsed()?;
            if elapsed - updated >= 1.0 {
                let water = converter.output_frames() as f64 / 44.1 - elapsed * 1000.0;
                let ppm = controller.update(water, elapsed - updated);
                converter.set_correction_ppm(ppm)?;
                updated = elapsed;
            }
        }
        Ok(())
    })
    .unwrap();
    converter.as_mut().unwrap().finish(&mut sink).unwrap();
    drop(tx);
    let delivered = consumer.join().unwrap();
    assert_eq!(delivered, converter.as_ref().unwrap().output_frames());
    assert!(controller.updates >= 25);
    assert!(controller.min_water_ms > 60.0 && controller.max_water_ms < 250.0);
    let report =
        serde_json::json!({"capture":capture,"controller":controller,"delivered_frames":delivered});
    fs::write(
        root.join("logs/live-drift-check.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("PASS real 默认音频来源 drift servo: {report}");
}

#[test]
#[ignore = "opens the real 默认音频来源 recording endpoint for 3 seconds"]
fn real_default_source_capture_conversion_and_bounded_delivery() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    let (tx, rx) = mpsc::sync_channel::<Vec<i16>>(64);
    let consumer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(64));
        let mut samples = 0;
        for block in rx {
            samples += block.len();
            thread::sleep(Duration::from_millis(10));
        }
        samples
    });
    let stop = AtomicBool::new(false);
    let mut converter = None;
    let mut sink = |pcm: &[i16]| -> Result<()> {
        tx.try_send(pcm.to_vec())
            .map_err(|_| "bounded delivery failed")?;
        Ok(())
    };
    let report = capture::live(&root, 3, &stop, |samples, rate| {
        if converter.is_none() {
            converter = Some(Converter::new(rate, 1234)?);
        }
        converter.as_mut().unwrap().push(samples, &mut sink)
    })
    .unwrap();
    converter.as_mut().unwrap().finish(&mut sink).unwrap();
    drop(tx);
    assert_eq!(consumer.join().unwrap(), 132300 * 2);
    assert_eq!(report["frames"], 144000);
    assert_eq!(report["discontinuities"], 0);
    assert_eq!(report["timestamp_errors"], 0);
    fs::write(
        root.join("logs/live-capture-check.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "PASS real 默认音频来源: 144000 captured frames -> 132300 PCM frames, bounded delivery"
    );
}
