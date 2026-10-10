//! GUI 控制约定和 CLI 取消守卫；控制消息与 PCM 音频分开。
//! GUI control contracts and CLI cancellation guard; control messages stay separate from PCM.
use std::{
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use windows::{Win32::System::Console::SetConsoleCtrlHandler, core::BOOL};
pub type GuiEmitter = Arc<dyn Fn(serde_json::Value) + Send + Sync>;
/// 页面命令与串流工作线程共用的控制面；音频数据不经这里传输。
/// mapping/volume 需要成组读写，使用 Mutex；停止及左右互换用原子标志通知。
/// Control plane shared by UI commands and the stream worker; audio does not pass through it.
/// Mutex protects grouped mapping/volume updates; atomic flags notify stop and speaker swapping.
pub struct GuiControl {
    pub stop: Arc<AtomicBool>,
    pub shutdown: crate::shutdown::Budget,
    /// False means detached cleanup still owns resources; the desktop must reserve this session.
    pub cleanup_complete: AtomicBool,
    pub mapping: Mutex<[usize; 2]>,
    pub speakers_swapped: AtomicBool,
    pub volume: Mutex<Vec<mpsc::SyncSender<String>>>,
}
impl GuiControl {
    pub fn new(mapping: [usize; 2]) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            shutdown: crate::shutdown::Budget::default(),
            cleanup_complete: AtomicBool::new(true),
            mapping: Mutex::new(mapping),
            speakers_swapped: AtomicBool::new(false),
            volume: Mutex::new(Vec::new()),
        }
    }
    pub fn request_stop(&self) {
        self.request_stop_until(std::time::Instant::now() + crate::shutdown::STOP_TIMEOUT);
    }
    pub fn request_stop_until(&self, deadline: std::time::Instant) {
        self.shutdown.begin_until(deadline);
        self.stop.store(true, Ordering::Release);
    }
}
/// 每次 GUI 连接的依赖；来源仍由桌面 Engine 持有，停止本次会话只解除订阅。
/// Per-connection GUI dependencies; the desktop Engine owns capture, so stopping only detaches.
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

pub(super) static STOP: AtomicBool = AtomicBool::new(false);
static STOP_BUDGET: LazyLock<Mutex<crate::shutdown::Budget>> = LazyLock::new(Mutex::default);
pub(super) fn stop_budget() -> crate::shutdown::Budget {
    STOP_BUDGET.lock().unwrap().clone()
}
unsafe extern "system" fn control(event: u32) -> BOOL {
    if event == 0 || event == 1 {
        stop_budget().begin();
        STOP.store(true, Ordering::Relaxed);
        BOOL(1)
    } else {
        BOOL(0)
    }
}
pub(super) struct Control;
impl Control {
    /// 仅 CLI 注册控制台处理器；守卫销毁时注销，GUI 使用独立会话停止标志。
    /// Register console control only for CLI; Drop unregisters it. GUI uses its session stop flag.
    pub(super) fn install(gui: bool) -> super::Result<Option<Self>> {
        if gui {
            return Ok(None);
        }
        STOP.store(false, Ordering::Relaxed);
        *STOP_BUDGET.lock().unwrap() = crate::shutdown::Budget::default();
        unsafe {
            SetConsoleCtrlHandler(Some(control), true)?;
        }
        Ok(Some(Self))
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(control), false);
        }
    }
}
