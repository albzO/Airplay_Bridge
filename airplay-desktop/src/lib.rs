//! Windows 桌面服务，供不同界面入口复用。
//! Shared desktop services for the Windows UI shells.
mod auth;
mod auth_memory;
mod awake;
mod configuration;
mod routing;
mod session;
mod settings;
mod startup;
mod stream_start;

use homepod_test::{capture, data_dir, discovery::Device, source::Source};
use serde::Serialize;
use serde_json::Value;
use session::Session;
pub use settings::{Appearance, Settings};
pub use startup::{is_preview_executable, set as set_autostart};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub enum DesktopEvent {
    SourceLevel(Value),
    Stream { session_id: u64, payload: Value },
}

/// 后台线程调用，界面入口应快速返回并另行调度渲染。
/// Called from background workers. UI adapters must return promptly and dispatch rendering separately.
pub type EventSink = Arc<dyn Fn(DesktopEvent) + Send + Sync>;

/// 已预留的配置操作。交给后台执行，或丢弃以释放占用。
/// An admitted configuration operation. Run on a worker, or drop it to release admission.
pub type DesktopJob<T> = Box<dyn FnOnce() -> Result<T, String> + Send>;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataMode {
    Portable,
    Installed,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Initialization {
    pub devices: Vec<Device>,
    pub inputs: Vec<capture::Input>,
    pub settings: Settings,
    pub data_path: PathBuf,
    pub data_mode: DataMode,
    pub backend_available: bool,
    pub awake_active: bool,
    pub awake_error: Option<String>,
    pub autostart: bool,
    pub autostart_error: Option<String>,
    pub capture_enabled: bool,
}

pub struct CaptureStatus {
    pub enabled: bool,
    pub present: bool,
    pub running: bool,
}

/// 持有持续 Source 和会话预留，直到收尾确认完成。
/// Owns the persistent Source and the current session reservation until cleanup completes.
pub struct Engine {
    root: PathBuf,
    portable: bool,
    backend: PathBuf,
    settings: Mutex<Settings>,
    auth_memory: Mutex<auth_memory::Memory>,
    session: Mutex<Option<Session>>,
    source: Mutex<Option<Arc<Source>>>,
    configuration_busy: AtomicBool,
    quitting: AtomicBool,
    capture_enabled: AtomicBool,
    awake: Mutex<awake::Awake>,
    awake_startup_error: Option<String>,
}

impl Engine {
    pub fn load() -> io::Result<Arc<Self>> {
        let root = data_dir::prepare()?;
        fs::create_dir_all(root.join("logs"))?;
        let exe = std::env::current_exe()?;
        let backend = exe.with_file_name("runtime").join("airplay-backend.exe");
        #[cfg(debug_assertions)]
        let backend = if backend.exists() {
            backend
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/runtime/airplay-backend.exe")
        };
        let settings = Settings::load(&root);
        let seed = backend.with_file_name("devices.json");
        let cache = root.join("devices.json");
        if !cache.exists() && seed.exists() {
            fs::copy(seed, cache)?;
        }
        let known_devices = read_devices(&root);
        let auth_memory =
            auth_memory::Memory::load(&root, &known_devices).map_err(io::Error::other)?;
        let mut awake = awake::Awake::default();
        let awake_startup_error = awake.set(settings.keep_awake).err();
        Ok(Arc::new(Self {
            root,
            portable: data_dir::portable_root(&exe).is_some(),
            backend,
            settings: Mutex::new(settings),
            auth_memory: Mutex::new(auth_memory),
            session: Mutex::new(None),
            source: Mutex::new(None),
            configuration_busy: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            capture_enabled: AtomicBool::new(false),
            awake: Mutex::new(awake),
            awake_startup_error,
        }))
    }

    /// 枚举 Windows 音频端点，在后台调用。
    /// Enumerates Windows endpoints. Call from a worker.
    pub fn initialize(&self) -> Result<Initialization, String> {
        let inputs = capture::enumerate().map_err(|e| e.to_string())?;
        let startup = startup::enabled();
        Ok(Initialization {
            devices: self.devices(),
            inputs,
            settings: self.settings.lock().unwrap().clone(),
            data_path: self.root.clone(),
            data_mode: if self.portable {
                DataMode::Portable
            } else {
                DataMode::Installed
            },
            backend_available: self.backend_available(),
            awake_active: self.awake.lock().unwrap().active(),
            awake_error: self.awake_startup_error.clone(),
            autostart: startup.as_ref().copied().unwrap_or(false),
            autostart_error: startup.err(),
            capture_enabled: self.capture_enabled.load(Ordering::SeqCst),
        })
    }

    pub fn data_root(&self) -> &Path {
        &self.root
    }

    pub fn backend_available(&self) -> bool {
        self.backend.exists()
    }

    pub fn devices(&self) -> Vec<Device> {
        read_devices(&self.root)
    }

    pub fn capture_status(&self) -> CaptureStatus {
        let source = self.source.lock().unwrap();
        CaptureStatus {
            enabled: self.capture_enabled.load(Ordering::SeqCst),
            present: source.is_some(),
            running: source.as_ref().is_some_and(|source| source.is_running()),
        }
    }

    pub fn submit_password(
        &self,
        session_id: u64,
        host: String,
        password: String,
    ) -> Result<(), String> {
        let reply = auth::Reply { host, password };
        let session = self.session.lock().unwrap();
        let s = session
            .as_ref()
            .filter(|s| s.id == session_id)
            .ok_or("会话已结束")?;
        if !s.hosts.contains(&reply.host)
            || reply.password.is_empty()
            || reply.password.len() > 1023
            || reply.password.contains(['\0', '\r', '\n'])
        {
            return Err("密码或设备无效".into());
        }
        s.reply
            .as_ref()
            .ok_or("密码通道尚未建立")?
            .send(reply)
            .map_err(|_| "密码请求已结束".into())
    }

    pub fn stop_stream(&self, session_id: u64) {
        session::stop(&self.session.lock().unwrap(), session_id);
    }

    pub fn set_volume(&self, percent: f64) -> Result<(), String> {
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err("音量超出范围".into());
        }
        let session = self.session.lock().unwrap();
        let s = session.as_ref().ok_or("尚未连接")?;
        let senders = s.control.volume.lock().unwrap();
        if senders.is_empty() {
            return Err("音量通道尚未建立".into());
        }
        for tx in senders.iter() {
            tx.try_send(format!("SET {percent:.1}\n"))
                .map_err(|_| "音量通道繁忙或已关闭")?;
        }
        Ok(())
    }

    pub fn open_logs(&self) -> Result<(), String> {
        std::process::Command::new("explorer.exe")
            .arg(self.root.join("logs"))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    pub fn quit_on_close(&self) -> bool {
        self.settings.lock().unwrap().close_action == "quit"
    }

    /// 只启动一次收尾。界面入口负责退出事件循环和进程级兜底。
    /// Begin cleanup once. The shell owns event-loop exit and its process-level fallback.
    pub fn begin_shutdown(self: &Arc<Self>) -> Option<(Instant, mpsc::Receiver<bool>)> {
        if self.quitting.swap(true, Ordering::SeqCst) {
            return None;
        }
        let deadline = Instant::now() + homepod_test::shutdown::STOP_TIMEOUT;
        let (done, completed) = mpsc::sync_channel(1);
        let engine = self.clone();
        thread::spawn(move || {
            if let Some(s) = engine.session.lock().unwrap().as_ref() {
                s.control.request_stop_until(deadline);
            }
            let _ = engine.awake.lock().unwrap().set(false);
            let source_stopped = stop_source_until(&engine, deadline).is_ok();
            while engine.session.lock().unwrap().is_some() && Instant::now() < deadline {
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(20)),
                );
            }
            let clean = source_stopped && engine.session.lock().unwrap().is_none();
            let _ = done.send(clean);
        });
        Some((deadline, completed))
    }
}

fn read_devices(root: &Path) -> Vec<Device> {
    fs::read(root.join("devices.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn stop_source(e: &Engine) -> Result<(), String> {
    stop_source_until(e, Instant::now() + homepod_test::shutdown::STOP_TIMEOUT)
}

fn stop_source_until(e: &Engine, deadline: Instant) -> Result<(), String> {
    let mut active = e.source.lock().unwrap();
    if let Some(source) = active.as_ref() {
        source.stop_until(deadline)?;
    }
    *active = None;
    Ok(())
}

/// 复用健康来源。新来源由调用者持有，完成检查后再发布。
/// Reuse a healthy source. The caller owns a new source until it can be safely published.
fn ensure_source(
    events: EventSink,
    e: &Engine,
    settings: &Settings,
    check: impl Fn() -> Result<(), String>,
) -> Result<(Arc<Source>, bool), String> {
    check()?;
    if e.quitting.load(Ordering::Relaxed) {
        return Err("应用正在退出".into());
    }
    let mut active = e.source.lock().unwrap();
    check()?;
    if let Some(source) = active.as_ref() {
        if source.endpoint == settings.endpoint && source.is_running() && !source.is_stopping() {
            *source.mapping.lock().unwrap() = settings.mapping;
            return Ok((source.clone(), false));
        }
    }
    if let Some(source) = active.as_ref() {
        source.stop()?;
    }
    *active = None;
    check()?;
    if settings.endpoint.is_empty() {
        return Err("未选择音频来源".into());
    }
    let emit = Arc::new(move |event| events(DesktopEvent::SourceLevel(event)));
    let source = Source::start(
        e.root.clone(),
        settings.endpoint.clone(),
        settings.mapping,
        emit,
    );
    Ok((source, true))
}
