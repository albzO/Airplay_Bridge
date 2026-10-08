//! 实时串流桥接：采集 → 有状态重采样 → 有界 PCM 队列 → 原生后端管道。
//! GUI 订阅持续 Source；CLI 使用自身采集流程，二者共用转换和协议调度。
//! 输出为 44.1 kHz、16 位、立体声 PCM；帧数每秒为 44100，字节数每秒为 176400。
//! 后端的 QPC 播放计划作为时基，控制器按缓冲水位微调采样率，避免长期时钟漂移。
//! 音频写管道、读协议日志、写诊断文件各自在线程中执行，不能阻塞采集回调。
//!
//! Live bridge: capture, stateful resampling, bounded PCM queue, then native backend pipe.
//! GUI subscribes to a persistent Source; CLI owns its capture path. Both share conversion
//! and protocol scheduling. Output is 44.1 kHz, 16-bit stereo: 44100 frames/176400 bytes per second.
//! The backend's QPC playback schedule is the timebase; water-level feedback corrects clock drift.
//! Pipe writing, protocol reading and diagnostic writing run on separate threads, not capture callbacks.
use crate::{capture, convert::Converter, discovery::Device, drift::Controller};
use std::os::windows::process::CommandExt;
use std::{
    error::Error,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::System::{Console::SetConsoleCtrlHandler, Performance::QueryPerformanceCounter},
    core::BOOL,
};
pub type GuiEmitter = Arc<dyn Fn(serde_json::Value) + Send + Sync>;
/// 页面命令与串流工作线程共用的控制面；音频数据不经这里传输。
/// mapping/volume 需要成组读写，使用 Mutex；停止及左右互换用原子标志通知。
/// Control plane shared by UI commands and the stream worker; audio does not pass through it.
/// Mutex protects grouped mapping/volume updates; atomic flags notify stop and speaker swapping.
pub struct GuiControl {
    pub stop: Arc<AtomicBool>,
    pub mapping: Mutex<[usize; 2]>,
    pub speakers_swapped: AtomicBool,
    pub volume: Mutex<Vec<mpsc::SyncSender<String>>>,
}
impl GuiControl {
    pub fn new(mapping: [usize; 2]) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            mapping: Mutex::new(mapping),
            speakers_swapped: AtomicBool::new(false),
            volume: Mutex::new(Vec::new()),
        }
    }
}
pub struct GuiContext {
    pub backend: PathBuf,
    pub endpoint: String,
    pub source: Arc<crate::source::Source>,
    pub password_pipe: String,
    pub password_first: bool,
    pub peer_password_first: bool,
    pub detailed_logs: bool,
    pub capture_diagnostics: bool,
    pub control: Arc<GuiControl>,
    pub emit: GuiEmitter,
}

// 磁盘写入移到独立线程：慢磁盘不能拖住采集，也不能让长测日志无限占用内存。
// 队列满时计数 dropped；finish 等待已入队记录落盘，报告保留丢记录信息。
// A separate disk worker keeps slow storage off the capture path and bounds logging memory.
// Count dropped entries on overflow; finish flushes queued records and reports those losses.
struct DetailLog {
    tx: mpsc::SyncSender<serde_json::Value>,
    dropped: AtomicU64,
    worker: thread::JoinHandle<std::io::Result<()>>,
}
impl DetailLog {
    fn start(path: &Path) -> std::io::Result<Self> {
        Self::start_bounded(path, 16, u64::MAX)
    }
    fn start_bounded(path: &Path, capacity: usize, limit: u64) -> std::io::Result<Self> {
        let mut file = File::create(path)?;
        let privacy = crate::privacy::Redactor::from_root(
            path.parent()
                .and_then(Path::parent)
                .unwrap_or(Path::new(".")),
        );
        let path = path.to_owned();
        let rolling = capacity > 16;
        let (tx, rx) = mpsc::sync_channel(capacity);
        let worker = thread::spawn(move || {
            let mut bytes = 0u64;
            let mut capped = false;
            let mut flushed = Instant::now();
            for entry in rx {
                if capped {
                    continue;
                }
                let line = format!("{}\n", privacy.value(&entry));
                if bytes + line.len() as u64 > limit {
                    if rolling {
                        file.flush()?;
                        drop(file);
                        rotate_capture_logs(&path)?;
                        file = File::create(&path)?;
                        bytes = 0;
                    } else {
                        capped = true;
                        continue;
                    }
                }
                file.write_all(line.as_bytes())?;
                bytes += line.len() as u64;
                if flushed.elapsed() >= Duration::from_millis(250) {
                    file.flush()?;
                    flushed = Instant::now();
                }
            }
            file.flush()?;
            if capped {
                return Err(std::io::Error::other(
                    "采集逐包诊断达到 128 MiB 上限，后续记录已省略",
                ));
            }
            Ok(())
        });
        Ok(Self {
            tx,
            dropped: AtomicU64::new(0),
            worker,
        })
    }
    fn record(&self, entry: serde_json::Value) {
        if self.tx.try_send(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn finish(self) -> serde_json::Value {
        drop(self.tx);
        let error = match self.worker.join() {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(_) => Some("详细日志线程异常".into()),
        };
        serde_json::json!({"dropped_records":self.dropped.load(Ordering::Relaxed),"error":error})
    }
}
// Keep current plus three previous segments, all local to this session.
fn rotate_capture_logs(path: &Path) -> std::io::Result<()> {
    let segment = |n| path.with_extension(format!("previous-{n}.jsonl"));
    if segment(3).exists() {
        fs::remove_file(segment(3))?;
    }
    for n in (1..3).rev() {
        if segment(n).exists() {
            fs::rename(segment(n), segment(n + 1))?;
        }
    }
    fs::rename(path, segment(1))
}
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn key_fault(line: &str) -> bool {
    [
        "[ERROR]",
        "[WARN]",
        "ERROR code=",
        "_FAILED",
        "PCM_STALL",
        "PCM_LATE",
        "VOLUME_REJECT",
        "RTSP channel failed",
    ]
    .iter()
    .any(|marker| line.contains(marker))
}
static STOP: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn control(event: u32) -> BOOL {
    if event == 0 || event == 1 {
        STOP.store(true, Ordering::Relaxed);
        BOOL(1)
    } else {
        BOOL(0)
    }
}
struct Control;
impl Drop for Control {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(control), false);
        }
    }
}
struct Backend(Child);
// Preserve the protocol failure that precedes the resulting broken PCM pipe.
fn backend_failure(line: &str) -> Option<String> {
    if line.contains("RTSP channel failed during ")
        || line.contains("[PROBE] PCM_STALL ")
        || line.contains("[PROBE] PCM_LATE ")
        || line.contains("[PROBE] EVENTS_FAILED ")
        || line.contains("[PROBE] GROUP_FAILED ")
    {
        Some(line.to_owned())
    } else {
        None
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
#[derive(Clone, Copy, serde::Serialize)]
struct Clock {
    start_qpc: u64,
    frequency: u64,
    prebuffer_frames: u64,
    lead_ms: u64,
}
impl Clock {
    fn parse(line: &str) -> Option<Self> {
        if !line.contains("[PROBE] PCM_CLOCK ") {
            return None;
        }
        let field = |name: &str| {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(name)?.parse::<u64>().ok())
        };
        let clock = Self {
            start_qpc: field("start_qpc=")?,
            frequency: field("frequency=")?,
            prebuffer_frames: field("prebuffer_frames=")?,
            lead_ms: field("lead_ms=")?,
        };
        if clock.frequency == 0 {
            None
        } else {
            Some(clock)
        }
    }
    fn elapsed(&self) -> Result<f64> {
        let mut counter = 0i64;
        unsafe {
            QueryPerformanceCounter(&mut counter)?;
        }
        Ok((counter as u64).saturating_sub(self.start_qpc) as f64 / self.frequency as f64)
    }
}
pub fn run(
    root: &Path,
    name: &str,
    seconds: u64,
    latency_ms: u32,
    buffer_ms: u32,
    endpoint: Option<&str>,
) -> Result<()> {
    run_targets(
        root, name, None, seconds, latency_ms, buffer_ms, None, endpoint,
    )
}
pub fn run_stereo(
    root: &Path,
    left: &str,
    right: &str,
    seconds: u64,
    latency_ms: u32,
    buffer_ms: u32,
    endpoint: Option<&str>,
) -> Result<()> {
    run_targets(
        root,
        left,
        Some(right),
        seconds,
        latency_ms,
        buffer_ms,
        None,
        endpoint,
    )
}
pub fn run_gui(
    root: &Path,
    name: &str,
    peer: Option<&str>,
    latency_ms: u32,
    buffer_ms: u32,
    gui: GuiContext,
) -> Result<()> {
    run_targets(root, name, peer, 0, latency_ms, buffer_ms, Some(gui), None)
}
fn run_targets(
    root: &Path,
    name: &str,
    peer_name: Option<&str>,
    seconds: u64,
    latency_ms: u32,
    buffer_ms: u32,
    gui: Option<GuiContext>,
    endpoint: Option<&str>,
) -> Result<()> {
    let devices: Vec<Device> = serde_json::from_slice(&fs::read(root.join("devices.json"))?)?;
    let matches: Vec<_> = devices.iter().filter(|d| d.name == name).collect();
    if matches.len() != 1 {
        return Err("设备名称不唯一或未发现，请先 discover".into());
    }
    let device = matches[0];
    let peer = if let Some(peer_name) = peer_name {
        let found: Vec<_> = devices.iter().filter(|d| d.name == peer_name).collect();
        if name == peer_name || found.len() != 1 {
            return Err("立体声需要两个不同且唯一的设备名称".into());
        }
        let other = found[0];
        let tsid = device.properties.get("tsid").filter(|s| !s.is_empty());
        if tsid.is_none() || tsid != other.properties.get("tsid") {
            return Err("两台设备没有相同的立体声组 tsid，请在家庭 App 核对并重新 discover".into());
        }
        Some(other)
    } else {
        None
    };
    let host = device
        .addresses
        .first()
        .ok_or("设备没有 IPv4 地址")?
        .to_string();
    let txt = device
        .properties
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let identity = format!("{stamp:016X}");
    let active_remote = ((stamp as u64 ^ (stamp as u64 >> 32)) as u32)
        .max(1)
        .to_string();
    fs::create_dir_all(root.join("logs"))?;
    let log_path = root.join("logs").join(format!("live-{stamp}.log"));
    let privacy = crate::privacy::Redactor::new(&devices);
    let mut log = File::create(&log_path)?;
    let detailed_logs = gui.as_ref().is_none_or(|g| g.detailed_logs);
    let mut drift_log = if detailed_logs {
        Some(File::create(log_path.with_extension("drift.jsonl"))?)
    } else {
        None
    };
    let dacp = crate::volume::Dacp::start_with_logging(
        *device.addresses.first().unwrap(),
        &identity,
        &active_remote,
        &log_path.with_extension("volume.log"),
        !detailed_logs,
    )?;
    let peer_identity = format!("{:016X}", stamp + 1);
    let peer_remote = active_remote
        .parse::<u32>()?
        .wrapping_add(1)
        .max(1)
        .to_string();
    let peer_dacp = if let Some(other) = peer {
        Some(crate::volume::Dacp::start_with_logging(
            *other.addresses.first().ok_or("另一台没有 IPv4 地址")?,
            &peer_identity,
            &peer_remote,
            &log_path.with_extension("peer.volume.log"),
            !detailed_logs,
        )?)
    } else {
        None
    };
    if let Some(g) = &gui {
        let mut senders = g.control.volume.lock().unwrap();
        senders.push(dacp.command_sender());
        if let Some(peer) = &peer_dacp {
            senders.push(peer.command_sender());
        }
        (g.emit)(
            serde_json::json!({"kind":"log_path","path":log_path,"detailed_logs":detailed_logs,"pipeline_path":if detailed_logs{Some(log_path.with_extension("pipeline.jsonl"))}else{None}}),
        );
    }
    let peer_args = if let Some(other) = peer {
        vec![
            "--peer-host".to_owned(),
            other.addresses[0].to_string(),
            "--peer-port".to_owned(),
            other.port.to_string(),
            "--peer-name".to_owned(),
            other.name.clone(),
            "--peer-txt".to_owned(),
            other
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" "),
            "--peer-identity".to_owned(),
            peer_identity,
            "--peer-active-remote".to_owned(),
            peer_remote,
            "--peer-volume-control-port".to_owned(),
            peer_dacp.as_ref().unwrap().control_port.to_string(),
        ]
    } else {
        Vec::new()
    };
    let stop = gui.as_ref().map_or(&STOP, |g| g.control.stop.as_ref());
    if gui.is_none() {
        STOP.store(false, Ordering::Relaxed);
    }
    if gui.is_none() {
        unsafe {
            SetConsoleCtrlHandler(Some(control), true)?;
        }
    }
    let _control = gui.is_none().then_some(Control);
    if seconds == 0 {
        println!(
            "建立持续流到 {name}（{host}:{}），持续运行直到 Ctrl+C。",
            device.port
        );
    } else {
        println!(
            "建立持续流到 {name}（{host}:{}），最多 {seconds} 秒。",
            device.port
        );
    }
    println!(
        "先自动认证；仅在设备要求密码时隐藏输入。连接完成后开始采集所选音频来源，Ctrl+C 正常停止。"
    );
    if let Some(other) = peer {
        println!(
            "立体声对：{name} + {}；若需要密码，只输入一次并用于两台，采集一次，共享 PTP 和播放起点。",
            other.name
        );
    }
    println!("日志：{}", log_path.display());
    let mut backend = Backend(
        Command::new(
            gui.as_ref()
                .map_or_else(|| crate::backend::executable(root), |g| g.backend.clone()),
        )
        .creation_flags(if gui.is_some() { 0x08000000 } else { 0 })
        .args([
            "--host",
            &host,
            "--port",
            &device.port.to_string(),
            "--name",
            name,
            "--txt",
            &txt,
            "--timing",
            "ptp",
            "--hold-seconds",
            "0",
            "--password-auto",
            "--pcm-stdin",
            "--identity",
            &identity,
            "--active-remote",
            &active_remote,
            "--volume-control-port",
            &dacp.control_port.to_string(),
            "--latency-ms",
            &latency_ms.to_string(),
            "--buffer-ms",
            &buffer_ms.to_string(),
        ])
        .args(&peer_args)
        .args(if detailed_logs {
            vec!["--debug"]
        } else {
            Vec::new()
        })
        .args(
            gui.as_ref()
                .map(|g| {
                    let mut args = vec!["--password-pipe".to_owned(), g.password_pipe.clone()];
                    if g.password_first {
                        args.push("--password-pipe-first".into());
                    }
                    if g.peer_password_first {
                        args.push("--peer-password-first".into());
                    }
                    args
                })
                .unwrap_or_default(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()?,
    );
    let stderr = backend.0.stderr.take().ok_or("缺少后端日志管道")?;
    let (ready_tx, ready_rx) = mpsc::channel();
    let transport = std::sync::Arc::new(AtomicBool::new(false));
    let transport_reader = transport.clone();
    let clock = Arc::new(Mutex::new(None::<Clock>));
    let clock_reader = clock.clone();
    let packet_status = Arc::new(Mutex::new(None::<String>));
    let member_packets = Arc::new(Mutex::new(
        std::collections::BTreeMap::<String, String>::new(),
    ));
    let member_reader = member_packets.clone();
    let volume_status = Arc::new(Mutex::new(None::<String>));
    let event_status = Arc::new(Mutex::new(None::<String>));
    let event_reader = event_status.clone();
    let failure_status = Arc::new(Mutex::new(None::<String>));
    let failure_reader = failure_status.clone();
    let auth_failure = Arc::new(Mutex::new(None::<crate::failure::SessionError>));
    let auth_reader = auth_failure.clone();
    let packet_reader = packet_status.clone();
    let volume_reader = volume_status.clone();
    let emit = gui.as_ref().map(|g| g.emit.clone());
    let faults_only = !detailed_logs;
    let reader_privacy = privacy.clone();
    let reader = thread::spawn(move || {
        let mut announced = false;
        let mut recent = std::collections::VecDeque::<String>::new();
        let mut context_written = false;
        let mut log_bytes = 0usize;
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            let safe_line = reader_privacy.text(&line);
            println!("{safe_line}");
            let is_fault = key_fault(&line);
            if !faults_only {
                let _ = writeln!(log, "{safe_line}");
            } else if is_fault && log_bytes < 262144 {
                if !context_written {
                    let _ = writeln!(log, "[CONTEXT] 最近的会话标记，仅在故障时保存");
                    for item in &recent {
                        let _ = writeln!(log, "{item}");
                        log_bytes += item.len() + 1;
                    }
                    context_written = true;
                }
                let bounded: String = safe_line.chars().take(2048).collect();
                let _ = writeln!(log, "{bounded}");
                log_bytes += bounded.len() + 1;
            }
            if faults_only {
                recent.push_back(safe_line.chars().take(1024).collect());
                if recent.len() > 24 {
                    recent.pop_front();
                }
            }
            if let Some(emit) = &emit {
                emit(
                    serde_json::json!({"kind":"native","line":line,"safe_line":safe_line,"is_fault":is_fault}),
                );
            }
            if let Some(error) = crate::failure::SessionError::parse(&line) {
                *auth_reader.lock().unwrap() = Some(error);
            }
            if let Some(failure) = backend_failure(&line) {
                let mut first = failure_reader.lock().unwrap();
                if first.is_none() {
                    *first = Some(failure);
                }
            }
            if line.contains("[PROBE] PACKET_STATS ") {
                *packet_reader.lock().unwrap() = Some(line.clone());
                if let Some(host) = line
                    .split_whitespace()
                    .find_map(|s| s.strip_prefix("host="))
                {
                    member_reader
                        .lock()
                        .unwrap()
                        .insert(host.to_owned(), line.clone());
                }
            }
            if line.contains("[PROBE] EVENTS_STATS ") {
                *event_reader.lock().unwrap() = Some(line.clone());
            }
            if line.contains("[PROBE] VOLUME_CURRENT ") || line.contains("[PROBE] VOLUME_APPLIED ")
            {
                *volume_reader.lock().unwrap() = Some(line.clone());
            }
            if let Some(origin) = Clock::parse(&line) {
                *clock_reader.lock().unwrap() = Some(origin);
            }
            if line.contains("[PROBE] PASSWORD_NEEDED ") {
                println!("设备要求 AirPlay 密码，请在本窗口隐藏输入并按回车。");
            }
            if line.contains("[PROBE] PCM_READY") && !announced {
                let _ = ready_tx.send(());
                announced = true;
            }
            if line.contains("[PROBE] AUDIO_TRANSPORT_OK") {
                transport_reader.store(true, Ordering::Relaxed);
            }
        }
    });
    let waiting = Instant::now();
    loop {
        match ready_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(()) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(error) = auth_failure.lock().unwrap().clone() {
                    return Err(error.into());
                }
                return Err("后端在音频流准备完成前退出，请查看日志".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if stop.load(Ordering::Relaxed) {
            return Err("已取消建立流".into());
        }
        if waiting.elapsed() > Duration::from_secs(90) {
            return Err("建立流或等待密码超过 90 秒".into());
        }
        if backend.0.try_wait()?.is_some() {
            let _ = reader.join();
            if let Some(error) = auth_failure.lock().unwrap().clone() {
                return Err(error.into());
            }
            return Err("后端连接失败，请查看日志".into());
        }
    }
    let mut pipe = backend.0.stdin.take().ok_or("缺少 PCM 管道")?;
    // Each converter block is roughly 10 ms. Queue caps storage at ~640 ms;
    // capture never waits on a blocked pipe writer or network.
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64);
    let pending_bytes = Arc::new(AtomicU64::new(0));
    let written_bytes = Arc::new(AtomicU64::new(0));
    let last_write_qpc = Arc::new(AtomicU64::new(0));
    let writer_pending = pending_bytes.clone();
    let writer_written = written_bytes.clone();
    let writer_qpc = last_write_qpc.clone();
    let writer = thread::spawn(move || -> std::io::Result<u64> {
        let mut bytes = 0;
        for block in rx {
            pipe.write_all(&block)?;
            bytes += block.len() as u64;
            writer_pending.fetch_sub(block.len() as u64, Ordering::Relaxed);
            writer_written.store(bytes, Ordering::Relaxed);
            let mut counter = 0i64;
            if unsafe { QueryPerformanceCounter(&mut counter) }.is_ok() {
                writer_qpc.store(counter as u64, Ordering::Relaxed);
            }
        }
        drop(pipe); // EOF requests tail silence and TEARDOWN.
        Ok(bytes)
    });
    let mut converter: Option<Converter> = None;
    let mut controller = Controller::new();
    let mut last_update = 0.0;
    let mut last_log = 0.0;
    let mut ahead_since: Option<Instant> = None;
    let mut sink = |pcm: &[i16]| -> Result<()> {
        let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let length = bytes.len() as u64;
        pending_bytes.fetch_add(length, Ordering::Relaxed);
        tx.try_send(bytes).map_err(|err| {
            pending_bytes.fetch_sub(length, Ordering::Relaxed);
            match err {
                mpsc::TrySendError::Full(_) => "PCM 队列已满，发送跟不上采集；已停止",
                mpsc::TrySendError::Disconnected(_) => "后端 PCM 管道已关闭",
            }
        })?;
        Ok(())
    };
    let mut ui_updated = Instant::now();
    let mut captured_frames = 0u64;
    let progress = Mutex::new(capture::CaptureProgress::default());
    let capture_diagnostics = gui.as_ref().is_some_and(|g| g.capture_diagnostics);
    let diagnostic_path = log_path.with_extension("capture.jsonl");
    let mut packet_log = if capture_diagnostics {
        Some(DetailLog::start_bounded(
            &diagnostic_path,
            1024,
            8 * 1024 * 1024,
        )?)
    } else {
        None
    };
    if let Some(g) = &gui {
        if capture_diagnostics {
            (g.emit)(
                serde_json::json!({"kind":"diagnostic_path","path":diagnostic_path,"pipeline_path":log_path.with_extension("pipeline.jsonl")}),
            );
        }
    }
    let mut packet_record = |mut entry: serde_json::Value| {
        entry["pending_pcm_ms"] =
            serde_json::json!(pending_bytes.load(Ordering::Relaxed) as f64 / 176.4);
        entry["pipe_written_frames"] = serde_json::json!(written_bytes.load(Ordering::Relaxed) / 4);
        if let Some(log) = &packet_log {
            log.record(entry);
        }
    };
    let mut trace = if detailed_logs || capture_diagnostics {
        Some(DetailLog::start(
            &log_path.with_extension("pipeline.jsonl"),
        )?)
    } else {
        None
    };
    if let Some(trace) = &trace {
        trace.record(serde_json::json!({"kind":"session","speakers_swapped":gui.as_ref().is_some_and(|g|g.control.speakers_swapped.load(Ordering::Relaxed)),"endpoint":gui.as_ref().map(|g|&g.endpoint),"mapping":gui.as_ref().map(|g|*g.control.mapping.lock().unwrap()),"output_rate":44100,"queue_blocks_max":64,"buffer_ms":buffer_ms,"latency_ms":latency_ms}));
    }
    let mut trace_updated = Instant::now();
    let mut peaks = [0f32; 2];
    let mut process_audio = |samples: &[f32], rate: u32| -> Result<()> {
        captured_frames += samples.len() as u64 / 2;
        if gui.is_some() {
            for frame in samples.chunks_exact(2) {
                for ch in 0..2 {
                    if frame[ch].is_finite() {
                        peaks[ch] = peaks[ch].max(frame[ch].abs());
                    }
                }
            }
        }
        if converter.is_none() {
            converter = Some(Converter::new(rate, stamp as u64)?);
        }
        let converter = converter.as_mut().unwrap();
        let routed;
        let samples = if peer.is_some()
            && gui
                .as_ref()
                .is_some_and(|g| g.control.speakers_swapped.load(Ordering::Relaxed))
        {
            routed = samples
                .chunks_exact(2)
                .flat_map(|f| [f[1], f[0]])
                .collect::<Vec<_>>();
            &routed[..]
        } else {
            samples
        };
        let mut converted = converter.push(samples, &mut sink);
        if let Some(origin) = *clock.lock().unwrap() {
            let water_ms = converter.output_frames() as f64 / 44.1 - origin.elapsed()? * 1000.0;
            if controller.updates >= 5 && water_ms > controller.target_ms + 120.0 {
                let since = ahead_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(5) {
                    converted = Err(format!("采集时间线持续超前：水位 {water_ms:.1} ms，目标 {:.1} ms；漂移修正无法消化，已停止以避免队列溢出",controller.target_ms).into());
                }
            } else {
                ahead_since = None;
            }
        }
        if let Some(trace) = &trace {
            if trace_updated.elapsed() >= Duration::from_secs(1) || converted.is_err() {
                let origin = *clock.lock().unwrap();
                let elapsed = origin.map(|o| o.elapsed()).transpose()?;
                let mut now = 0i64;
                unsafe {
                    QueryPerformanceCounter(&mut now)?;
                }
                let last_write = last_write_qpc.load(Ordering::Relaxed);
                trace.record(serde_json::json!({"kind":if converted.is_err(){"fault"}else{"sample"},"qpc":now,"elapsed_seconds":elapsed,"input_rate":rate,"capture_frames":captured_frames,"input_audio_seconds":captured_frames as f64/rate as f64,"conversion":converter.stats,"output_audio_seconds":converter.output_frames() as f64/44100.0,"water_ms":elapsed.map(|s|converter.output_frames() as f64/44.1-s*1000.0),"pending_pcm_ms":pending_bytes.load(Ordering::Relaxed) as f64/176.4,"pipe_written_frames":written_bytes.load(Ordering::Relaxed)/4,"writer_idle_ms":origin.filter(|_|last_write>0).map(|o|(now as u64).saturating_sub(last_write) as f64*1000.0/o.frequency as f64),"capture":*progress.lock().unwrap(),"controller":controller,"member_packet_stats":*member_packets.lock().unwrap(),"error":converted.as_ref().err().map(ToString::to_string)}));
                if capture_diagnostics {
                    if let Some(g) = &gui {
                        (g.emit)(
                            serde_json::json!({"kind":"capture_diagnostic","elapsed_seconds":elapsed,"capture_frames":captured_frames,"pending_pcm_ms":pending_bytes.load(Ordering::Relaxed) as f64/176.4,"water_ms":elapsed.map(|s|converter.output_frames() as f64/44.1-s*1000.0),"capture":*progress.lock().unwrap(),"correction_ppm":controller.correction_ppm,"error":converted.as_ref().err().map(ToString::to_string)}),
                        );
                    }
                }
                trace_updated = Instant::now();
            }
        }
        converted?;
        if let Some(origin) = *clock.lock().unwrap() {
            let elapsed = origin.elapsed()?;
            if let Some(g) = &gui {
                if ui_updated.elapsed() >= Duration::from_millis(250) {
                    (g.emit)(
                        serde_json::json!({"kind":"telemetry","elapsed_seconds":elapsed,
                    "input_rate":rate,"capture_frames":captured_frames,"output_frames":converter.output_frames(),
                    "water_ms":converter.output_frames() as f64/44.1-elapsed*1000.0,
                    "lead_ms":origin.lead_ms,"peaks":peaks,"controller":controller}),
                    );
                    peaks = [0.; 2];
                    ui_updated = Instant::now();
                }
            }
            if elapsed - last_update >= 1.0 {
                // Whole pipeline water level: all produced samples minus the
                // backend's fixed QPC-paced schedule. Includes queue/pipe/prebuffer.
                let water_ms = converter.output_frames() as f64 / 44.1 - elapsed * 1000.0;
                let ppm = controller.update(water_ms, elapsed - last_update);
                converter.set_correction_ppm(ppm)?;
                last_update = elapsed;
                if elapsed - last_log >= 10.0 {
                    let entry = serde_json::json!({"elapsed_seconds":elapsed,"water_ms":water_ms,
                        "target_ms":controller.target_ms,"filtered_ms":controller.filtered_ms,"correction_ppm":ppm,
                        "lead_ms":origin.lead_ms,"estimated_audio_delay_ms":origin.lead_ms as f64+water_ms});
                    if let Some(log) = &mut drift_log {
                        writeln!(log, "{entry}")?;
                    }
                    println!(
                        "[DRIFT] 缓冲={water_ms:.1}ms 目标={:.1}ms 校正={ppm:.1}ppm lead={}ms",
                        controller.target_ms, origin.lead_ms
                    );
                    last_log = elapsed;
                }
            }
        }
        Ok(())
    };
    let mut capture_result = if let Some(g) = &gui {
        g.source.consume(
            stop,
            &progress,
            if capture_diagnostics {
                Some(&mut packet_record)
            } else {
                None
            },
            &mut process_audio,
        )
    } else {
        capture::live_selected_diagnosed(
            root,
            seconds,
            stop,
            endpoint,
            None,
            (detailed_logs || capture_diagnostics).then_some(&progress),
            if capture_diagnostics {
                Some(&mut packet_record)
            } else {
                None
            },
            None,
            &mut process_audio,
        )
    };
    if capture_result.is_ok() {
        if let Some(converter) = &mut converter {
            if let Err(err) = converter.finish(&mut sink) {
                capture_result = Err(err);
            }
        }
    }
    drop(tx);
    if let Some(trace) = &trace {
        trace.record(serde_json::json!({"kind":"capture_end","capture_frames":captured_frames,"conversion":converter.as_ref().map(|c|&c.stats),"capture":*progress.lock().unwrap(),"pending_pcm_ms":pending_bytes.load(Ordering::Relaxed) as f64/176.4,"pipe_written_frames":written_bytes.load(Ordering::Relaxed)/4,"error":capture_result.as_ref().err().map(ToString::to_string),"stopped_by_user":stop.load(Ordering::Relaxed)}));
    }
    let trace_status = trace.take().map(DetailLog::finish);
    if let Some(log) = &packet_log {
        log.record(serde_json::json!({"kind":"capture_end","capture_frames":captured_frames,"error":capture_result.as_ref().err().map(ToString::to_string),"stopped_by_user":stop.load(Ordering::Relaxed)}));
    }
    let packet_log_status = packet_log.take().map(DetailLog::finish);
    if let Some(g) = &gui {
        if capture_diagnostics {
            (g.emit)(serde_json::json!({"kind":"diagnostic_end","status":packet_log_status}));
        }
    }
    if capture_result.is_ok() {
        println!("采集已停止，正在发送剩余音频并关闭会话……");
    } else {
        println!("串流异常中断，正在收集后端退出信息……");
    }
    let closing = Instant::now();
    let status = loop {
        if let Some(status) = backend.0.try_wait()? {
            break status;
        }
        if closing.elapsed() > Duration::from_secs(15) {
            backend.0.kill()?;
            backend.0.wait()?;
            let _ = writer.join();
            let _ = reader.join();
            return Err("后端停止超过 15 秒，已终止；请查看日志".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let written = writer.join().map_err(|_| "PCM 写入线程异常")?;
    let _ = reader.join();
    let backend_error = failure_status.lock().unwrap().clone();
    let report = serde_json::json!({"device":name,"requested_seconds":seconds,
        "peer_device":peer.map(|d|&d.name),"shared_ptp":peer.is_some(),"speakers_swapped":gui.as_ref().is_some_and(|g|g.control.speakers_swapped.load(Ordering::Relaxed)),
        "member_packet_stats":*member_packets.lock().unwrap(),
        "capture":capture_result.as_ref().ok(),
        "capture_error":capture_result.as_ref().err().map(|e| e.to_string()),
        "conversion":converter.as_ref().map(|c| &c.stats),
        "pipe_bytes":written.as_ref().ok(),"pipe_error":written.as_ref().err().map(|e|crate::failure::describe(&format!("PCM 发送管道：{e}"), "PCM_PIPE_FAILED")),
        "queue_blocks_max":64,"clock":*clock.lock().unwrap(),"requested_latency_ms":latency_ms,"requested_buffer_ms":buffer_ms,
        "backend_exit":status.code(),"transport_ok":transport.load(Ordering::Relaxed),
        "backend_error":backend_error,
        "last_event_stats":*event_status.lock().unwrap(),
        "drift_correction":true,"drift_controller":controller,
        "last_packet_stats":*packet_status.lock().unwrap(),"last_volume_marker":*volume_status.lock().unwrap(),
        "volume_log":log_path.with_extension("volume.log"),
        "peer_volume_log":peer.map(|_|log_path.with_extension("peer.volume.log")),
        "delay_estimate_scope":"sender pipeline + protocol lead; excludes application/physical rendering/acoustic delay",
        "capture_log_mode":"rolling_4_segments_8MiB","capture_diagnostics":capture_diagnostics,"capture_trace":if capture_diagnostics{Some(diagnostic_path)}else{None},"capture_trace_status":packet_log_status,
        "detailed_logs":detailed_logs,"pipeline_trace":if detailed_logs || capture_diagnostics{Some(log_path.with_extension("pipeline.jsonl"))}else{None},"pipeline_trace_status":trace_status,
        "drift_trace":if detailed_logs{Some(log_path.with_extension("drift.jsonl"))}else{None}});
    let report_path = log_path.with_extension("json");
    let report = privacy.value(&report);
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    if let Some(g) = &gui {
        (g.emit)(serde_json::json!({"kind":"report","report":report}));
    }
    println!("流报告：{}", report_path.display());
    if let Some(cause) = backend_error {
        return Err(format!("后端串流中断：{cause}；日志：{}", log_path.display()).into());
    }
    capture_result?;
    written
        .map_err(|e| crate::failure::describe(&format!("PCM 发送管道：{e}"), "PCM_PIPE_FAILED"))?;
    if !status.success() || !transport.load(Ordering::Relaxed) {
        return Err("持续流发送失败，请查看日志".into());
    }
    println!("持续流发送完成；请核对 HomePod 内容、连续性和停止是否正常。");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detailed_trace_flushes_samples_and_terminal_fault() {
        let path =
            std::env::temp_dir().join(format!("airplay-detail-{}.jsonl", std::process::id()));
        let trace = DetailLog::start(&path).unwrap();
        trace.record(serde_json::json!({"kind":"sample","pending_pcm_ms":150.0}));
        trace.record(serde_json::json!({"kind":"fault","error":"PCM 队列已满"}));
        let status = trace.finish();
        assert_eq!(status["dropped_records"], 0);
        assert!(status["error"].is_null());
        let contents = fs::read_to_string(&path).unwrap();
        let entries: Vec<serde_json::Value> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1]["kind"], "fault");
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn diagnostic_file_limit_preserves_valid_records_and_reports_truncation() {
        let path = std::env::temp_dir().join(format!(
            "airplay-diagnostic-cap-{}.jsonl",
            std::process::id()
        ));
        let log = DetailLog::start_bounded(&path, 16, 64).unwrap();
        log.record(serde_json::json!({"kind":"start"}));
        log.record(serde_json::json!({"kind":"packet","padding":"x".repeat(100)}));
        let status = log.finish();
        assert_eq!(status["dropped_records"], 0);
        assert!(status["error"].as_str().unwrap().contains("上限"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.len() <= 64);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text.trim()).unwrap()["kind"],
            "start"
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn rolling_diagnostic_keeps_latest_fault_and_bounds_old_segments() {
        let directory = std::env::temp_dir().join(format!("airplay-rotate-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("capture.jsonl");
        let log = DetailLog::start_bounded(&path, 1024, 64).unwrap();
        for n in 0..20 {
            log.record(serde_json::json!({"packet":n,"payload":"1234567890"}));
        }
        log.record(serde_json::json!({"kind":"fault","error":"capture stopped"}));
        let status = log.finish();
        assert!(status["error"].is_null());
        assert_eq!(status["dropped_records"], 0);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("capture stopped")
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 4);
        for file in fs::read_dir(&directory).unwrap() {
            let file = file.unwrap();
            assert!(file.metadata().unwrap().len() <= 64);
            for line in fs::read_to_string(file.path()).unwrap().lines() {
                serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
            fs::remove_file(file.path()).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn backend_failure_distinguishes_disconnect_from_success_and_tolerated_miss() {
        assert!(backend_failure("[ERROR] [AP2] RTSP channel failed during POST /feedback read after 0ms: connection reset; terminating native session").is_some());
        assert!(backend_failure("[PROBE] PCM_STALL timeout_ms=1000").is_some());
        assert!(backend_failure("[PROBE] PCM_LATE deadline_ms=1234").is_some());
        assert!(backend_failure("[PROBE] EVENTS_FAILED reason=authentication").is_some());
        assert!(
            backend_failure(
                "[WARN] POST /feedback keepalive miss 1/3; tolerating transient failure"
            )
            .is_none()
        );
        assert!(backend_failure("[PROBE] PCM_EOF frames=44100").is_none());
        assert!(backend_failure("[PROBE] AUDIO_TRANSPORT_OK").is_none());
    }
    #[test]
    fn clock_marker_uses_shared_qpc_and_rejects_invalid_frequency() {
        let mut now = 0;
        unsafe {
            QueryPerformanceCounter(&mut now).unwrap();
        }
        let line = format!(
            "[PROBE] PCM_CLOCK start_qpc={now} frequency=10000000 prebuffer_frames=5984 lead_ms=1000"
        );
        let clock = Clock::parse(&line).unwrap();
        assert!(clock.elapsed().unwrap() < 1.0);
        assert!(
            Clock::parse("[PROBE] PCM_CLOCK start_qpc=1 frequency=0 prebuffer_frames=1 lead_ms=1")
                .is_none()
        );
    }
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
                windows::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
                    .unwrap();
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
        let report = serde_json::json!({"capture":capture,"controller":controller,"delivered_frames":delivered});
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
}
