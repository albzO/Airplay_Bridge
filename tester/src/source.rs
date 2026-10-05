//! One WASAPI owner. Attaching a stream never closes or reopens capture.
use crate::capture::{self, CaptureProgress};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
enum Event {
    Audio(Vec<f32>, u32, CaptureProgress, Instant),
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "reads real B3 and attaches/detaches two short consumers; no HomePod"]
    fn real_source_keeps_capture_running_across_two_stream_consumers() {
        let input = capture::enumerate()
            .unwrap()
            .into_iter()
            .find(|i| {
                i.flow == "recording"
                    && i.name
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .any(|word| word == "B3")
            })
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
        let source = Arc::new(Source {
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
        });
        for value in [0.25, 0.75] {
            let producer = source.clone();
            let worker = thread::spawn(move || {
                let started = Instant::now();
                loop {
                    if let Some(sub) = producer.subscriber.lock().unwrap().as_ref() {
                        sub.tx
                            .try_send(Event::Audio(
                                vec![value, value],
                                48000,
                                CaptureProgress::default(),
                                Instant::now(),
                            ))
                            .unwrap();
                        let _ = sub.tx.try_send(Event::Audio(
                            vec![9., 9.],
                            48000,
                            CaptureProgress::default(),
                            Instant::now(),
                        ));
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
}
struct Subscription {
    tx: mpsc::SyncSender<Event>,
    fault: Arc<Mutex<Option<String>>>,
    diagnostic_tx: Option<mpsc::SyncSender<Value>>,
    diagnostic_drops: Arc<AtomicU64>,
}
pub struct Source {
    pub endpoint: String,
    pub mapping: Mutex<[usize; 2]>,
    stop: AtomicBool,
    done: AtomicBool,
    subscriber: Mutex<Option<Subscription>>,
    progress: Mutex<CaptureProgress>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    error: Mutex<Option<String>>,
    info: Mutex<Value>,
    diagnostic_enabled: AtomicBool,
}
impl Source {
    pub fn start(
        root: PathBuf,
        endpoint: String,
        mapping: [usize; 2],
        emit: Arc<dyn Fn(Value) + Send + Sync>,
    ) -> Arc<Self> {
        let source = Arc::new(Self {
            endpoint,
            mapping: Mutex::new(mapping),
            stop: AtomicBool::new(false),
            done: AtomicBool::new(false),
            subscriber: Mutex::new(None),
            progress: Mutex::new(CaptureProgress::default()),
            worker: Mutex::new(None),
            error: Mutex::new(None),
            info: Mutex::new(Value::Null),
            diagnostic_enabled: AtomicBool::new(false),
        });
        let s = source.clone();
        let worker = thread::spawn(move || {
            let mut peaks = [0f32; 2];
            let mut updated = Instant::now();
            let mut diagnostic = |entry: Value| {
                if entry["kind"] == "capture_start" {
                    *s.info.lock().unwrap() = entry.clone();
                }
                if entry["kind"] == "gap_repaired" {
                    emit(
                        json!({"endpoint":s.endpoint,"warning":format!("采集缺口 {:.1} ms，已补静音继续",entry["gap_ms"].as_f64().unwrap_or(0.))}),
                    );
                }
                let subscriber = s.subscriber.lock().unwrap();
                if let Some(sub) = subscriber.as_ref() {
                    if let Some(tx) = &sub.diagnostic_tx {
                        if tx.try_send(entry).is_err() {
                            sub.diagnostic_drops.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            };
            let result = capture::live_selected_diagnosed(
                &root,
                0,
                &s.stop,
                Some(&s.endpoint),
                Some(&s.mapping),
                Some(&s.progress),
                Some(&mut diagnostic),
                Some(&s.diagnostic_enabled),
                |samples, rate| {
                    for frame in samples.chunks_exact(2) {
                        for ch in 0..2 {
                            if frame[ch].is_finite() {
                                peaks[ch] = peaks[ch].max(frame[ch].abs());
                            }
                        }
                    }
                    if updated.elapsed() >= Duration::from_millis(100) {
                        emit(json!({"endpoint":s.endpoint,"peaks":peaks}));
                        peaks = [0.; 2];
                        updated = Instant::now();
                    }
                    let mut subscriber = s.subscriber.lock().unwrap();
                    if let Some(sub) = subscriber.as_ref() {
                        let event = Event::Audio(
                            samples.to_vec(),
                            rate,
                            *s.progress.lock().unwrap(),
                            Instant::now(),
                        );
                        if let Err(error) = sub.tx.try_send(event) {
                            if matches!(error, mpsc::TrySendError::Full(_)) {
                                *sub.fault.lock().unwrap() = Some(
                                    "持续采集分支队列已满，处理跟不上采集；电平预览继续运行".into(),
                                );
                            }
                            *subscriber = None;
                            s.diagnostic_enabled.store(false, Ordering::Relaxed);
                        }
                    }
                    Ok(())
                },
            );
            let error = result.err().map(|e| e.to_string());
            *s.error.lock().unwrap() = error.clone();
            if let Some(sub) = s.subscriber.lock().unwrap().take() {
                *sub.fault.lock().unwrap() = error.clone();
            }
            s.done.store(true, Ordering::Release);
            emit(json!({"endpoint":s.endpoint,"peaks":[0.,0.],"error":error}));
        });
        *source.worker.lock().unwrap() = Some(worker);
        source
    }
    pub fn is_running(&self) -> bool {
        !self.done.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
    pub fn consume(
        &self,
        stop: &AtomicBool,
        progress: &Mutex<CaptureProgress>,
        mut diagnostic: Option<&mut dyn FnMut(Value)>,
        mut sink: impl FnMut(&[f32], u32) -> Result<()>,
    ) -> Result<Value> {
        let (tx, rx) = mpsc::sync_channel(64);
        let (diagnostic_tx, diagnostic_rx) = mpsc::sync_channel(1024);
        let diagnostic_drops = Arc::new(AtomicU64::new(0));
        let fault = Arc::new(Mutex::new(None));
        let initial = *self.progress.lock().unwrap();
        {
            let mut subscriber = self.subscriber.lock().unwrap();
            if subscriber.is_some() {
                return Err("已有串流使用持续采集器".into());
            }
            if !self.is_running() {
                return Err(self
                    .error
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or("持续采集器已结束".into())
                    .into());
            }
            *subscriber = Some(Subscription {
                tx,
                fault: fault.clone(),
                diagnostic_tx: diagnostic.is_some().then_some(diagnostic_tx),
                diagnostic_drops: diagnostic_drops.clone(),
            });
        }
        self.diagnostic_enabled
            .store(diagnostic.is_some(), Ordering::Relaxed);
        if let Some(log) = diagnostic.as_mut() {
            log(
                json!({"capture_info":*self.info.lock().unwrap(),"kind":"stream_attach","endpoint":self.endpoint,"capture_position":initial}),
            );
        }
        let mut frames = 0u64;
        let result = (|| -> Result<()> {
            while !stop.load(Ordering::Relaxed) {
                if let Some(error) = fault.lock().unwrap().clone() {
                    return Err(error.into());
                }
                for entry in diagnostic_rx.try_iter().take(64) {
                    if let Some(log) = diagnostic.as_mut() {
                        log(entry);
                    }
                }
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(Event::Audio(samples, rate, position, received)) => {
                        *progress.lock().unwrap() = position;
                        let processing = Instant::now();
                        let backlog_ms = received.elapsed().as_secs_f64() * 1000.0;
                        let result = sink(&samples, rate);
                        if let Some(log) = diagnostic.as_mut() {
                            log(
                                json!({"kind":"consumer_delivery","frames":samples.len()/2,"source_backlog_ms":backlog_ms,"consumer_ms":processing.elapsed().as_secs_f64()*1000.0,"capture_position":position,"error":result.as_ref().err().map(ToString::to_string)}),
                            );
                        }
                        result?;
                        frames += samples.len() as u64 / 2;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => {
                        return Err(fault
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or("持续采集器已结束".into())
                            .into());
                    }
                }
            }
            Ok(())
        })();
        *self.subscriber.lock().unwrap() = None;
        self.diagnostic_enabled.store(false, Ordering::Relaxed);
        for entry in diagnostic_rx.try_iter() {
            if let Some(log) = diagnostic.as_mut() {
                log(entry);
            }
        }
        let final_position = *self.progress.lock().unwrap();
        if let Some(log) = diagnostic.as_mut() {
            log(
                json!({"kind":"stream_detach","diagnostic_drops":diagnostic_drops.load(Ordering::Relaxed),"frames":frames,"capture_position":final_position,"error":result.as_ref().err().map(ToString::to_string)}),
            );
        }
        result?;
        Ok(
            json!({"capture_info":*self.info.lock().unwrap(),"mode":"continuous_shared","endpoint":self.endpoint,"frames":frames,"discontinuities":final_position.discontinuities.saturating_sub(initial.discontinuities),"timestamp_errors":final_position.timestamp_errors.saturating_sub(initial.timestamp_errors),"repaired_gaps":final_position.repaired_gaps.saturating_sub(initial.repaired_gaps),"repaired_gap_frames":final_position.repaired_gap_frames.saturating_sub(initial.repaired_gap_frames),"capture_position":final_position,"stopped_by_user":stop.load(Ordering::Relaxed)}),
        )
    }
}
