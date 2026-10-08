//! 持续采集来源：一个 Source 独占一条 WASAPI 采集线程。
//! 同一来源同时提供页面预览和一个串流订阅；连接/断开订阅不重启采集。
//! 采集线程不能等待网络或磁盘。音频通过有界队列交付，积压时报告故障，
//! 防止内存无限增长；诊断队列允许丢记录，并在收尾报告丢弃数量。
//!
//! Each Source owns one WASAPI capture thread, serving preview and one stream subscriber.
//! Attaching/detaching a stream does not restart capture. Capture never waits on network or disk.
//! Bounded audio queues report overflow as a fault; diagnostic queues may drop records and
//! report the count during cleanup instead of allowing unbounded memory growth.
use crate::capture::{self, CaptureProgress};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
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
#[derive(Default)]
/// 稳定性看数据包连续到达，不看振幅：系统在空闲时生成的静音也算有效数据。
/// 故障或包间隔超过 250 ms 会重置连续窗口；500 ms 稳定且至少 3 包才可连接。
/// Readiness measures continuous packet delivery, not amplitude; intentional idle silence is valid.
/// Faults or gaps over 250 ms reset the window; require 500 ms stability and at least three packets.
struct Warmup {
    since: Option<Instant>,
    last: Option<Instant>,
    packets: u64,
    faults: (u64, u64, u64),
}
impl Warmup {
    fn observe(&mut self, now: Instant, progress: CaptureProgress) {
        let faults = (
            progress.discontinuities,
            progress.timestamp_errors,
            progress.repaired_gaps,
        );
        if faults != self.faults
            || self
                .last
                .is_some_and(|last| now.duration_since(last) > Duration::from_millis(250))
        {
            self.since = None;
        }
        self.since.get_or_insert(now);
        self.last = Some(now);
        self.faults = faults;
        self.packets += 1;
    }
    fn ready(&self, now: Instant) -> bool {
        self.since
            .is_some_and(|since| now.duration_since(since) >= Duration::from_millis(500))
            && self
                .last
                .is_some_and(|last| now.duration_since(last) <= Duration::from_millis(250))
            && self.packets >= 3
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
    #[ignore = "reads real default playback endpoint startup for two seconds; no HomePod"]
    fn real_playback_startup_progress() {
        let input = capture::enumerate()
            .unwrap()
            .into_iter()
            .find(|i| i.id == capture::default_endpoint("playback").unwrap())
            .unwrap();
        let root = std::env::current_dir().unwrap().join("build/source-checks");
        let source = Source::start(root, input.id, [0, 1], Arc::new(|_| {}));
        source.wait_ready(&AtomicBool::new(false)).unwrap();
        thread::sleep(Duration::from_secs(2));
        let progress = *source.progress.lock().unwrap();
        println!(
            "default playback endpoint startup progress: {}",
            serde_json::to_string(&progress).unwrap()
        );
        assert!(source.is_running());
        source.stop();
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
            warmup: Mutex::new(Warmup::default()),
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
    warmup: Mutex<Warmup>,
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
            warmup: Mutex::new(Warmup::default()),
        });
        let s = source.clone();
        let worker = thread::spawn(move || {
            let privacy = crate::privacy::Redactor::from_root(&root);
            let directory = root.join("logs");
            let path = directory.join("source-startup.jsonl");
            let _ = fs::create_dir_all(&directory);
            if fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
                let backup = directory.join("source-startup.previous.jsonl");
                let _ = fs::remove_file(&backup);
                let _ = fs::rename(&path, backup);
            }
            let startup_log = Mutex::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .ok(),
            );
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let started = Instant::now();
            let write_startup = |entry: Value| {
                if let Some(file) = startup_log.lock().unwrap().as_mut() {
                    let _ = writeln!(
                        file,
                        "{}",
                        json!({"source_id":stamp,"endpoint":s.endpoint,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"entry":privacy.value(&entry)})
                    );
                }
            };
            write_startup(json!({"kind":"source_requested","mapping":*s.mapping.lock().unwrap()}));
            let mut peaks = [0f32; 2];
            let mut updated = Instant::now();
            let mut summary_updated = Instant::now();
            let mut summary_peaks = [0f32; 2];
            let mut diagnostic = |entry: Value| {
                if entry["kind"] == "capture_start" {
                    *s.info.lock().unwrap() = entry.clone();
                    write_startup(entry.clone());
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
                    let progress = *s.progress.lock().unwrap();
                    if !samples.is_empty() && rate > 0 {
                        s.warmup.lock().unwrap().observe(Instant::now(), progress);
                    }
                    for frame in samples.chunks_exact(2) {
                        for ch in 0..2 {
                            if frame[ch].is_finite() {
                                peaks[ch] = peaks[ch].max(frame[ch].abs());
                                summary_peaks[ch] = summary_peaks[ch].max(frame[ch].abs());
                            }
                        }
                    }
                    if started.elapsed() < Duration::from_secs(11)
                        && summary_updated.elapsed() >= Duration::from_secs(1)
                    {
                        write_startup(
                            json!({"kind":"startup_progress","rate":rate,"peaks":summary_peaks,"capture":progress,"ready":s.warmup.lock().unwrap().ready(Instant::now())}),
                        );
                        summary_peaks = [0.; 2];
                        summary_updated = Instant::now();
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
            let error = result
                .err()
                .map(|e| crate::failure::describe(&e.to_string(), "CAPTURE_INIT_FAILED"));
            write_startup(
                json!({"kind":"source_finished","error":error,"capture":*s.progress.lock().unwrap()}),
            );
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
    /// 网络握手前等待连续采集稳定，最多等待 8 秒；取消和采集线程故障立即返回。
    /// 静音不是故障，不能以“有声音”作为就绪条件，否则无声来源永远无法连接。
    /// Wait for stable capture before handshaking, at most eight seconds; cancel/faults return early.
    /// Silence is valid: requiring audible content would prevent quiet sources from ever connecting.
    pub fn wait_ready(&self, cancelled: &AtomicBool) -> Result<()> {
        let started = Instant::now();
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err("启动采集已取消".into());
            }
            if !self.is_running() {
                return Err(self
                    .error
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or("采集器已结束".into())
                    .into());
            }
            if self.warmup.lock().unwrap().ready(Instant::now()) {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(8) {
                return Err(
                    "采集管线未能稳定：8 秒内未收到连续采集数据，请检查或切换音频来源".into(),
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    /// 结束整个采集线程并等待退出；只在切换来源、关闭采集或退出应用时调用。
    /// Stop and join capture only when switching sources, disabling capture or exiting the app.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
    /// 订阅后续采集数据，sink 接收交错排列的 float32 左右声道及来源采样率。
    /// 仅允许一个订阅者；64 个音频块的队列与 1024 条诊断队列各自限量。
    /// stop 只结束本次订阅，保留来源线程；统计以挂接时的计数作基线，避免把
    /// 连接前预览阶段的故障算进本次报告。sink 出错也会先解除订阅再返回。
    /// Subscribe to future interleaved float32 stereo samples and their source sample rate.
    /// Allow one subscriber; bound audio at 64 blocks and diagnostics at 1024 records.
    /// stop ends this subscription, not capture. Use attach-time counters as the report baseline
    /// to exclude earlier preview faults. Detach before returning even when sink fails.
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
