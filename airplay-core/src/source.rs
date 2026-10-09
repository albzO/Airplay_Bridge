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
const LOOPBACK_REOPEN_LIMIT: u32 = 3;
enum Event {
    Audio(Vec<f32>, u32, CaptureProgress, Instant),
}
#[derive(Default)]
/// 稳定性看数据包连续到达，不看振幅：系统在空闲时生成的静音也算有效数据。
/// 故障或包间隔超过 250 ms 会重置连续窗口；500 ms 稳定且至少 3 包才可连接。
/// 首次采集稳定前，播放端点电平与原始包相矛盾时暂停计时；稳定后不因暂停重新开启检查。
/// Readiness measures continuous packet delivery, not amplitude; intentional idle silence is valid.
/// Faults or gaps over 250 ms reset the window; require 500 ms stability and at least three packets.
/// Meter/raw-PCM contradictions suspend startup warmup only; pausing established capture does not rearm the check.
struct Warmup {
    since: Option<Instant>,
    last: Option<Instant>,
    packets: u64,
    faults: (u64, u64, u64),
}
impl Warmup {
    fn observe(&mut self, now: Instant, progress: CaptureProgress) {
        // 包连续到达不代表取得真实音频；端点电平与原始包矛盾时不得宣告就绪。
        // Continuous packets do not guarantee valid audio; contradictory endpoint/raw PCM suspends readiness.
        if progress.loopback_suspect {
            *self = Self::default();
            return;
        }
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
            self.packets = 0;
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
        let (tx, _rx) = mpsc::sync_channel(64);
        *source.subscriber.lock().unwrap() = Some(Subscription {
            tx,
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
                        privacy.value(&json!({"source_id":stamp,"endpoint":s.endpoint,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"entry":entry}))
                    );
                }
            };
            write_startup(
                json!({"kind":"source_requested","mapping":*s.mapping.lock().unwrap(),"process_id":std::process::id(),"loopback_health_check":"startup_endpoint_vs_raw_pcm_v2"}),
            );
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
            let capture = || {
                capture::live_selected_diagnosed(
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
                                        "持续采集分支队列已满，处理跟不上采集；电平预览继续运行"
                                            .into(),
                                    );
                                }
                                *subscriber = None;
                                s.diagnostic_enabled.store(false, Ordering::Relaxed);
                            }
                        }
                        Ok(())
                    },
                )
            };
            let result = s.capture_with_recovery(capture, |attempt, previous| {
                write_startup(json!({"kind":"loopback_reopen","attempt":attempt,"limit":LOOPBACK_REOPEN_LIMIT,"reason":"active_output_zero_raw_pcm","capture":previous}));
                emit(json!({"endpoint":s.endpoint,"peaks":[0.,0.],"warning":format!("播放回环未取得音频，正在重新初始化采集（{attempt}/{LOOPBACK_REOPEN_LIMIT}）")}));
            });
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
    /// 复用 Source 对象但释放整套 WASAPI 客户端后重开，等价于采集开关的资源重置。
    /// 只在尚未订阅串流时恢复，最多三次；已订阅时保留故障，避免重置时钟造成撕裂。
    /// Reuse Source but reopen WASAPI after releasing the old client, matching the capture toggle's reset.
    /// Recover only before subscription, at most three times; preserve live faults rather than reset a running clock.
    fn capture_with_recovery(
        &self,
        mut capture: impl FnMut() -> Result<Value>,
        mut reopened: impl FnMut(u32, CaptureProgress),
    ) -> Result<Value> {
        let mut attempts = 0;
        loop {
            let result = capture();
            if !result
                .as_ref()
                .err()
                .is_some_and(|error| error.is::<capture::LoopbackStalled>())
            {
                return result;
            }
            // 与订阅登记共用这把锁：重建期间必须先撤销就绪，再允许新的串流登记。
            // Share the subscription lock: revoke readiness before a new stream can attach during reopening.
            let subscriber = self.subscriber.lock().unwrap();
            if subscriber.is_some() || attempts >= LOOPBACK_REOPEN_LIMIT {
                return result;
            }
            let previous = *self.progress.lock().unwrap();
            *self.warmup.lock().unwrap() = Warmup::default();
            *self.progress.lock().unwrap() = CaptureProgress::default();
            drop(subscriber);
            if self.stop.load(Ordering::Relaxed) {
                return Ok(Value::Null);
            }
            attempts += 1;
            reopened(attempts, previous);
            // 给旧客户端释放后的音频引擎 160 ms；按 20 ms 检查停止，关闭采集无需等完整重试。
            // Allow 160 ms after client release; poll stop every 20 ms so disabling capture cancels recovery promptly.
            for _ in 0..8 {
                if self.stop.load(Ordering::Relaxed) {
                    return Ok(Value::Null);
                }
                thread::sleep(Duration::from_millis(20));
            }
            if self.stop.load(Ordering::Relaxed) {
                return Ok(Value::Null);
            }
        }
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
            let suspect = self.progress.lock().unwrap().loopback_suspect;
            if !suspect && self.warmup.lock().unwrap().ready(Instant::now()) {
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
        // 网络认证期间预览采集也可能重建，挂接时必须重新核对当前就绪状态。
        // Preview can reopen during network authentication; recheck readiness when attaching.
        let initial = loop {
            self.wait_ready(stop)?;
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
            if !self.warmup.lock().unwrap().ready(Instant::now()) {
                continue;
            }
            let initial = *self.progress.lock().unwrap();
            if initial.loopback_suspect {
                continue;
            }
            *subscriber = Some(Subscription {
                tx,
                fault: fault.clone(),
                diagnostic_tx: diagnostic.is_some().then_some(diagnostic_tx),
                diagnostic_drops: diagnostic_drops.clone(),
            });
            break initial;
        };
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
