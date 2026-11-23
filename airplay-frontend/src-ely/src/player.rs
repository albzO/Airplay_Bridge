//! 播放器状态与操作：来源、接收端、会话、密码、音量和设置保存。界面在 view.rs 与 settings_view.rs。
//! Player state and actions: source, receiver, session, password, volume and settings.
//! Views live in view.rs and settings_view.rs.
use crate::{
    desktop_job,
    diagnostics::Diagnostics,
    elastic::Elastic,
    inbox::{Inbox, Pending, StreamUpdate},
    lang::{self, t},
    session::{Phase, Session},
    shell,
    source_picker::Shift,
    tf,
    visibility::{self, Step},
    volume::VolumeReport,
};
use airplay_desktop::{Appearance, DesktopJob, Engine, Initialization, Settings, set_autostart};
use ely_core::{
    forms::{InputEvent, TextInput},
    theme::{Mode, Theme},
};
use gpui::{AppContext, Context, Entity, FocusHandle, Task, Window};
use homepod_test::{capture::Input, discovery::Device};
use std::{
    cell::Cell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use windows::Win32::Foundation::HWND;

/// 水位曲线保留的采样数。
/// Samples kept for the water-level sparkline.
const WATER_HISTORY: usize = 48;
/// 可见性变化后稍等再判断，避免一闪而过的最小化反复开关音频设备。
/// Settle before acting on a visibility change, so a brief minimize does not cycle the device.
const VISIBILITY_SETTLE: Duration = Duration::from_millis(400);
/// 恢复采集失败后的重试：显示器音频端点在亮屏后要过一会儿才重新出现。
/// Retries for resuming capture: a monitor's audio endpoint takes a moment to return after the
/// display turns on.
const RESUME_RETRIES: u8 = 3;
const RESUME_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Playback,
    General,
    Runtime,
    Technical,
    Logs,
}

impl Page {
    pub const TABS: [Page; 4] = [Page::General, Page::Runtime, Page::Technical, Page::Logs];

    pub fn key(self) -> &'static str {
        match self {
            Page::Playback => "playback",
            Page::General => "general",
            Page::Runtime => "runtime",
            Page::Technical => "technical",
            Page::Logs => "logs",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Page::Playback]
            .into_iter()
            .chain(Self::TABS)
            .find(|page| page.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::Playback => t("播放", "Playback"),
            Page::General => t("常规", "General"),
            Page::Runtime => t("运行统计", "Statistics"),
            Page::Technical => t("技术详情", "Technical"),
            Page::Logs => t("日志", "Logs"),
        }
    }
}

/// 正在进行的后台作业；界面据此显示状态并禁用相关控件。
/// The worker job in flight; the view shows it as status and disables the affected controls.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Initializing,
    ReadingInputs,
    SavingSource,
    PreparingPreview,
    TogglingCapture,
    PausingCapture,
    ResumingCapture,
    SavingSettings,
    Autostart,
    ForgettingAuth,
}

impl Op {
    pub fn label(self) -> &'static str {
        match self {
            Op::Initializing => t("正在初始化", "Starting up"),
            Op::ReadingInputs => t("正在读取音频设备", "Reading audio devices"),
            Op::SavingSource => t("正在保存来源", "Saving source"),
            Op::PreparingPreview => t("正在准备电平预览", "Preparing level preview"),
            Op::TogglingCapture => t("正在切换采集", "Switching capture"),
            Op::PausingCapture => t("正在暂停采集", "Pausing capture"),
            Op::ResumingCapture => t("正在恢复采集", "Resuming capture"),
            Op::SavingSettings => t("正在保存设置", "Saving settings"),
            Op::Autostart => t("正在更新开机自启", "Updating start at login"),
            Op::ForgettingAuth => t("正在清除密码要求记录", "Clearing password memory"),
        }
    }
}

/// 一张接收端卡片：立体声对（同 tsid 恰好两台）或单台设备。
/// One receiver card: a stereo pair (exactly two members sharing tsid) or a single device.
pub struct Receiver {
    pub key: String,
    pub label: String,
    pub names: Vec<String>,
    /// 成员在 `Player::devices` 中的下标，igl 主设备在前。
    /// Member indices into `Player::devices`, the igl primary first.
    pub members: Vec<usize>,
}

fn receivers(devices: &[Device]) -> Vec<Receiver> {
    let mut groups = BTreeMap::<&str, Vec<usize>>::new();
    for (index, device) in devices.iter().enumerate() {
        if let Some(group) = device.properties.get("tsid").filter(|id| !id.is_empty()) {
            groups.entry(group).or_default().push(index);
        }
    }
    let mut list = Vec::new();
    for (group, mut members) in groups.into_iter().filter(|(_, members)| members.len() == 2) {
        members.sort_by_key(|&index| {
            let device = &devices[index];
            (
                device.properties.get("igl").map(String::as_str) != Some("1"),
                &device.service,
            )
        });
        let names: Vec<_> = members.iter().map(|&i| devices[i].name.clone()).collect();
        list.push(Receiver {
            key: format!("pair:{group}"),
            label: names.join(" + "),
            names,
            members,
        });
    }
    list.extend(devices.iter().enumerate().map(|(index, device)| Receiver {
        key: format!("device:{}", device.service),
        label: device.name.clone(),
        names: vec![device.name.clone()],
        members: vec![index],
    }));
    list
}

pub struct Player {
    pub(crate) engine: Arc<Engine>,
    inbox: Inbox,
    pub(crate) focus: FocusHandle,
    pub(crate) settings: Settings,
    pub(crate) inputs: Vec<Input>,
    pub(crate) devices: Vec<Device>,
    pub(crate) receivers: Vec<Receiver>,
    pub(crate) selected: Option<String>,
    pub(crate) expanded: Option<String>,
    pub(crate) operation: Option<Op>,
    pub(crate) language: lang::Choice,
    pub(crate) backend_available: bool,
    /// 正在发现设备。与其他作业并行，只锁住设备卡片、刷新和开始串流。
    /// Discovering devices. Runs beside other jobs and locks only the device cards, refresh and
    /// starting a stream.
    pub(crate) discovering: bool,
    pub(crate) capture_enabled: bool,
    auto_capture_pending: bool,
    hwnd: Option<HWND>,
    /// 窗口当前能否被看到，见 visibility.rs。
    /// Whether the window can be seen now; see visibility.rs.
    seen: bool,
    /// 采集因窗口不可见而被停掉，恢复可见时重新开启。
    /// Capture was stopped because the window was unseen, and restarts when it is seen again.
    paused: bool,
    /// 暂停失败后不再重试，直到可见性再次变化。
    /// After a failed pause, do not retry until visibility changes again.
    pause_failed: bool,
    resume_task: Option<Task<()>>,
    pub(crate) page: Page,
    pub(crate) scroll: Elastic,
    /// 音频来源下拉：是否展开、框的位置、列表的滚动与本帧位移。
    /// The audio source picker: whether it is open, the box's bounds, the list's scroll and this
    /// frame's shift.
    pub(crate) source_open: bool,
    pub(crate) source_anchor: Rc<Cell<gpui::Bounds<gpui::Pixels>>>,
    pub(crate) source_scroll: Elastic,
    pub(crate) source_shift: Shift,
    /// 每次得到新的设备列表加一，设备卡片据此重新播放出现动画。
    /// Bumped with every new device list, so the device cards play their entrance again.
    pub(crate) devices_generation: usize,
    pub(crate) last_tab: Page,
    pub(crate) diagnostics: Diagnostics,
    pub(crate) water: VecDeque<f32>,
    last_elapsed: Option<f64>,
    pub(crate) auth_notice: Option<String>,
    pub(crate) autostart: bool,
    /// 拖动中的音量目标；接收端确认后清空，滑块回到确认值。
    /// The volume being dragged; cleared once the receiver confirms, so the slider shows the
    /// confirmed level again.
    pub(crate) volume_draft: Option<f64>,
    volume_task: Option<Task<()>>,
    pub(crate) peaks: [f32; 2],
    pub(crate) message: Option<String>,
    pub(crate) source_message: Option<String>,
    pub(crate) session: Session,
    pub(crate) password: Entity<TextInput>,
    retry_secret: Option<String>,
}

impl Player {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (inbox, wake) = Inbox::new();
        let incoming = inbox.clone();
        cx.spawn_in(window, async move |view, cx| {
            while wake.recv().await.is_ok() {
                let pending = incoming.take();
                if cx
                    .update(|window, cx| {
                        view.update(cx, |player, cx| player.accept(pending, window, cx))
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let password = cx.new(|cx| {
            TextInput::new(window, cx)
                .masked()
                .max_len(1023)
                .placeholder(t("输入 AirPlay 密码", "Enter the AirPlay password"))
        });
        cx.subscribe_in(&password, window, |player, _, event, window, cx| {
            if matches!(event, InputEvent::Submit) {
                player.submit_password(window, cx);
            }
        })
        .detach();
        cx.observe_window_appearance(window, |player, window, cx| {
            if player.settings.appearance == Appearance::System {
                player.apply_appearance(window, cx);
            }
        })
        .detach();
        let visible = visibility::watch();
        cx.spawn_in(window, async move |view, cx| {
            while visible.recv().await.is_ok() {
                cx.background_executor().timer(VISIBILITY_SETTLE).await;
                while visible.try_recv().is_ok() {}
                let updated = cx.update(|window, cx| {
                    view.update(cx, |player, cx| player.visibility_changed(window, cx))
                });
                if !matches!(updated, Ok(Ok(()))) {
                    break;
                }
            }
        })
        .detach();
        // 最小化与还原没有单独的事件，借激活和尺寸变化触发重新判断。
        // Minimize and restore have no event of their own; activation and bounds changes prompt
        // the check.
        cx.observe_window_activation(window, |_, _, _| visibility::changed())
            .detach();
        cx.observe_window_bounds(window, |_, _, _| visibility::changed())
            .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.on_next_frame(window, |player, window, cx| player.initialize(window, cx));
        let language = lang::load(engine.data_root());
        Self {
            engine,
            inbox,
            focus,
            settings: Settings::default(),
            inputs: Vec::new(),
            devices: Vec::new(),
            receivers: Vec::new(),
            selected: None,
            expanded: None,
            operation: Some(Op::Initializing),
            language,
            backend_available: false,
            discovering: false,
            capture_enabled: false,
            auto_capture_pending: true,
            hwnd: shell::native_window(window).ok(),
            seen: true,
            paused: false,
            pause_failed: false,
            resume_task: None,
            page: Page::Playback,
            scroll: Elastic::default(),
            source_open: false,
            source_anchor: Rc::default(),
            source_scroll: Elastic::default(),
            source_shift: Rc::default(),
            devices_generation: 0,
            last_tab: Page::General,
            diagnostics: Diagnostics::default(),
            water: VecDeque::new(),
            last_elapsed: None,
            auth_notice: None,
            autostart: false,
            volume_draft: None,
            volume_task: None,
            peaks: [0.; 2],
            message: None,
            source_message: None,
            session: Session::default(),
            password,
            retry_secret: None,
        }
    }

    fn run_job<T: Send + 'static>(
        &mut self,
        label: Op,
        job: Result<DesktopJob<T>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
        complete: impl FnOnce(&mut Self, Result<T, String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let result = match job.and_then(desktop_job::dispatch) {
            Ok(result) => result,
            Err(error) => {
                self.operation = None;
                self.message = Some(error);
                cx.notify();
                return;
            }
        };
        self.operation = Some(label);
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result = result.recv().await.unwrap_or_else(|_| {
                Err(t("桌面任务提前退出", "The desktop task exited early").into())
            });
            let _ = cx.update(|window, cx| {
                view.update(cx, |player, cx| {
                    player.operation = None;
                    complete(player, result, window, cx);
                    // 作业期间可见性可能已变化。
                    // Visibility may have changed while the job ran.
                    player.settle_capture(window, cx);
                    cx.notify();
                })
            });
        })
        .detach();
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        self.run_job(
            Op::ReadingInputs,
            Ok(Box::new(move || engine.initialize())),
            window,
            cx,
            Self::initialized,
        );
    }

    fn initialized(
        &mut self,
        result: Result<Initialization, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = match result {
            Ok(state) => state,
            Err(error) => {
                self.message = Some(error);
                return;
            }
        };
        self.inputs = state.inputs;
        self.inputs.sort_by_key(|source| source.name.to_lowercase());
        self.settings = state.settings;
        self.backend_available = state.backend_available;
        self.capture_enabled = state.capture_enabled;
        self.autostart = state.autostart;
        self.message = state.autostart_error;
        self.diagnostics.set_mode(state.data_mode);
        self.apply_appearance(window, cx);
        self.set_devices(state.devices);
        self.selected = self.receivers.first().map(|receiver| receiver.key.clone());
        if !self.backend_available {
            self.message = Some(
                t(
                    "缺少原生后端，请从完整的便携目录启动。",
                    "The native backend is missing. Start from the complete app folder.",
                )
                .into(),
            );
        }
        // 发现设备与电平预览同时开始，互不等待。
        // Discovery and the level preview start together; neither waits for the other.
        if self.backend_available {
            self.refresh(window, cx);
        }
        if self.source().is_some() {
            self.auto_capture_pending = false;
            self.preview(window, cx);
        }
    }

    pub(crate) fn source(&self) -> Option<&Input> {
        self.inputs
            .iter()
            .find(|source| source.id == self.settings.endpoint && source.channels.unwrap_or(0) > 0)
    }

    pub(crate) fn receiver(&self) -> Option<&Receiver> {
        let key = self.selected.as_ref()?;
        self.receivers.iter().find(|receiver| &receiver.key == key)
    }

    fn set_devices(&mut self, devices: Vec<Device>) {
        self.devices_generation += 1;
        self.receivers = receivers(&devices);
        self.devices = devices;
        if self.receiver().is_none() {
            // 原选择已消失（或首次发现）：退回第一张卡片，立体声对排在最前。
            // The old choice is gone (or this is the first discovery): fall back to the first
            // card; stereo pairs sort first.
            self.selected = self.receivers.first().map(|receiver| receiver.key.clone());
            self.session = Session::default();
        }
    }

    /// 来源、接收端和串流相关操作此时不可用：有后台作业、会话占用或正在退出。
    /// Source, receiver and stream actions are unavailable: a job is running, a session holds
    /// them, or the app is quitting.
    pub(crate) fn locked(&self) -> bool {
        self.operation.is_some() || self.session.id.is_some() || self.engine.is_quitting()
    }

    /// 已保存设置此时不可改；连接建立后（Ready / Playing）允许修改。
    /// Saved settings cannot change now; once connected (Ready / Playing) they can.
    pub(crate) fn settings_locked(&self) -> bool {
        self.operation.is_some()
            || self.engine.is_quitting()
            || (self.session.id.is_some()
                && !matches!(self.session.phase, Phase::Ready | Phase::Playing))
    }

    pub(crate) fn connected(&self) -> bool {
        matches!(self.session.phase, Phase::Ready | Phase::Playing)
    }

    /// 设备卡片、刷新和开始串流此时不可用。
    /// The device cards, refresh and starting a stream are unavailable now.
    pub(crate) fn devices_locked(&self) -> bool {
        self.locked() || self.discovering
    }

    pub(crate) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.discovering || self.session.id.is_some() || self.engine.is_quitting() {
            return;
        }
        let result = match self
            .engine
            .prepare_discover_devices()
            .and_then(desktop_job::dispatch)
        {
            Ok(result) => result,
            Err(error) => {
                self.message = Some(error);
                cx.notify();
                return;
            }
        };
        self.discovering = true;
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result = result.recv().await.unwrap_or_else(|_| {
                Err(t("桌面任务提前退出", "The desktop task exited early").into())
            });
            let _ = view.update(cx, |player, cx| {
                player.discovering = false;
                match result {
                    Ok(devices) => player.set_devices(devices),
                    Err(error) => player.message = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn select_source(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() || self.settings.endpoint == id {
            return;
        }
        let Some(source) = self.inputs.iter().find(|source| source.id == id) else {
            return;
        };
        let mut next = self.settings.clone();
        next.endpoint = source.id.clone();
        // 换来源后旧声道下标可能越界，先恢复合法映射。
        // Old channel indices may be out of range for the new source; reset to a valid mapping.
        next.mapping = [0, usize::from(source.channels.unwrap_or(0) > 1)];
        self.message = None;
        let job = self.engine.prepare_save_settings(next.clone());
        self.run_job(
            Op::SavingSource,
            job,
            window,
            cx,
            move |player, result, window, cx| match result {
                Ok(()) => {
                    player.settings = next;
                    player.peaks = [0.; 2];
                    player.source_message = None;
                    if player.capture_wanted() || player.auto_capture_pending {
                        player.paused = false;
                        player.resume_task = None;
                        player.auto_capture_pending = false;
                        player.preview(window, cx);
                    }
                }
                Err(error) => player.message = Some(error),
            },
        );
    }

    fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sink = self.inbox.sink();
        let job = if self.capture_enabled {
            self.engine.prepare_monitor_source(sink)
        } else {
            self.engine.prepare_set_capture_enabled(sink, true)
        };
        self.run_job(
            Op::PreparingPreview,
            job,
            window,
            cx,
            |player, result, _, _| match result {
                Ok(()) => player.capture_enabled = true,
                Err(error) => player.source_message = Some(error),
            },
        );
    }

    pub(crate) fn toggle_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() || self.source().is_none() {
            return;
        }
        self.auto_capture_pending = false;
        self.source_message = None;
        self.message = None;
        let enabled = !self.capture_wanted();
        self.paused = false;
        self.resume_task = None;
        let job = self
            .engine
            .prepare_set_capture_enabled(self.inbox.sink(), enabled);
        self.run_job(
            Op::TogglingCapture,
            job,
            window,
            cx,
            move |player, result, _, _| match result {
                Ok(()) => {
                    player.capture_enabled = enabled;
                    if !enabled {
                        player.peaks = [0.; 2];
                    }
                }
                Err(error) => player.message = Some(error),
            },
        );
    }

    /// 用户希望采集开启；暂停或等待重试时仍为真。
    /// The user wants capture on; still true while paused or waiting to retry.
    pub(crate) fn capture_wanted(&self) -> bool {
        self.capture_enabled || self.paused || self.resume_task.is_some()
    }

    fn visibility_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let seen = self.hwnd.is_none_or(visibility::seen);
        if seen == self.seen {
            return;
        }
        self.seen = seen;
        self.pause_failed = false;
        if seen && self.capture_enabled && !self.locked() {
            // 没来得及暂停，采集器可能已在熄屏时出错；健康的会直接复用。
            // Not paused in time, so the source may have faulted with the display off; a healthy
            // one is simply reused.
            self.resume_capture(RESUME_RETRIES, window, cx);
        } else {
            self.settle_capture(window, cx);
        }
    }

    /// 未串流时让采集跟随窗口可见性；有作业或会话时不动，结束后会再次调用。
    /// While idle, make capture follow visibility. Left alone during a job or session; called
    /// again when they end.
    fn settle_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locked = self.locked();
        if !locked && !self.seen && self.resume_task.take().is_some() && !self.capture_enabled {
            self.paused = true;
        }
        let capturing = self.capture_enabled && !self.pause_failed;
        match visibility::next_step(self.seen, locked, capturing, self.paused) {
            Some(Step::Pause) => {
                let job = self
                    .engine
                    .prepare_set_capture_enabled(self.inbox.sink(), false);
                self.run_job(
                    Op::PausingCapture,
                    job,
                    window,
                    cx,
                    |player, result, _, _| match result {
                        Ok(()) => {
                            player.capture_enabled = false;
                            player.paused = true;
                            player.peaks = [0.; 2];
                            player.source_message = None;
                        }
                        Err(error) => {
                            player.pause_failed = true;
                            player.source_message = Some(error);
                        }
                    },
                );
            }
            Some(Step::Resume) => self.resume_capture(RESUME_RETRIES, window, cx),
            None => {}
        }
    }

    /// 重新开启采集并等它就绪；失败时隔一会儿重试。
    /// Turns capture back on and waits until it is ready, retrying after a pause on failure.
    fn resume_capture(&mut self, retries: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.paused = false;
        self.resume_task = None;
        let enabling = !self.capture_enabled;
        let sink = self.inbox.sink();
        let job = if enabling {
            self.engine.prepare_set_capture_enabled(sink, true)
        } else {
            self.engine.prepare_monitor_source(sink)
        };
        self.run_job(
            Op::ResumingCapture,
            job,
            window,
            cx,
            move |player, result, window, cx| match result {
                Ok(()) if enabling => {
                    player.capture_enabled = true;
                    player.resume_capture(retries, window, cx);
                }
                Ok(()) => player.source_message = None,
                Err(error) if retries == 0 => player.source_message = Some(error),
                Err(_) => {
                    player.resume_task = Some(cx.spawn_in(window, async move |view, cx| {
                        cx.background_executor().timer(RESUME_RETRY_DELAY).await;
                        let _ = cx.update(|window, cx| {
                            view.update(cx, |player, cx| {
                                if player.seen && !player.locked() {
                                    player.resume_capture(retries - 1, window, cx);
                                } else {
                                    // 别的作业占用中，交给它结束后的 settle_capture。
                                    // Another job holds the engine; settle_capture follows it.
                                    player.resume_task = None;
                                    player.paused = !player.capture_enabled;
                                }
                            })
                        });
                    }));
                }
            },
        );
    }

    pub(crate) fn choose_receiver(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.discovering {
            return;
        }
        if !self.locked() && self.selected.as_deref() != Some(key) {
            self.selected = Some(key.to_owned());
            self.session = Session::default();
            self.message = None;
            self.clear_password(cx);
        }
        self.expanded = if self.expanded.as_deref() == Some(key) {
            None
        } else {
            Some(key.to_owned())
        };
        let _ = window;
        cx.notify();
    }

    pub(crate) fn start(
        &mut self,
        retry_secret: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.devices_locked() || self.source().is_none() || !self.backend_available {
            return;
        }
        let Some(names) = self.receiver().map(|receiver| receiver.names.clone()) else {
            return;
        };
        self.message = None;
        // start_stream 同步返回预留编号。界面下一次处理队列前编号已登记。
        // start_stream returns the reservation synchronously, before the view drains the inbox.
        match self.engine.start_stream(
            self.inbox.session_sink(self.settings.detailed_logs),
            names,
            self.settings.clone(),
            retry_secret.is_some(),
        ) {
            Ok(id) => {
                self.session.start(id);
                self.paused = false;
                self.resume_task = None;
                self.diagnostics.begin(&self.settings);
                self.water.clear();
                self.last_elapsed = None;
                self.volume_task = None;
                self.volume_draft = None;
                self.expanded = self.selected.clone();
                if retry_secret.is_none() {
                    self.clear_password(cx);
                }
                self.retry_secret = retry_secret;
            }
            Err(error) => self.message = Some(error),
        }
        let _ = window;
        cx.notify();
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.session.id {
            self.session.stop();
            self.volume_task = None;
            self.volume_draft = None;
            self.retry_secret = None;
            self.clear_password(cx);
            self.engine.stop_stream(id);
            cx.notify();
        }
    }

    pub(crate) fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.session.prompt.clone() else {
            return;
        };
        let secret = self.password.read(cx).text().to_owned();
        if secret.is_empty() {
            return;
        }
        if let Some(id) = self.session.id {
            self.send_password(id, host, secret, cx);
        } else {
            self.start(Some(secret), window, cx);
        }
    }

    fn send_password(&mut self, id: u64, host: String, secret: String, cx: &mut Context<Self>) {
        match self.engine.submit_password(id, host.clone(), secret) {
            Ok(()) => self.session.password_sent(host),
            Err(error) => self.message = Some(error),
        }
        cx.notify();
    }

    fn clear_password(&mut self, cx: &mut Context<Self>) {
        self.password.update(cx, |input, cx| input.set_text("", cx));
    }

    /// 密码提示所指设备的名称；主机地址不在已发现设备中时退回通用称呼。
    /// Name of the device the password prompt is for, or a generic label for an unknown host.
    pub(crate) fn prompt_name(&self) -> Option<String> {
        let host = self.session.prompt.as_ref()?;
        Some(
            self.devices
                .iter()
                .find(|device| device.addresses.iter().any(|a| a.to_string() == *host))
                .map_or(t("当前接收端", "this receiver"), |device| {
                    device.name.as_str()
                })
                .to_owned(),
        )
    }

    fn accept(&mut self, pending: Pending, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.id {
            self.diagnostics
                .accept(pending.diagnostics, id, &self.devices);
            let telemetry = &self.diagnostics.telemetry;
            if telemetry.elapsed_seconds != self.last_elapsed
                && let Some(water) = telemetry.water_ms
            {
                self.last_elapsed = telemetry.elapsed_seconds;
                if self.water.len() == WATER_HISTORY {
                    self.water.pop_front();
                }
                self.water.push_back(water as f32);
            }
        }
        if let Some(source) = pending
            .source
            .filter(|update| update.endpoint == self.settings.endpoint)
        {
            if let Some(enabled) = source.capture_enabled {
                self.capture_enabled = enabled;
            }
            if let Some(peaks) = source.peaks {
                self.peaks = peaks;
            }
            if let Some(warning) = &source.warning {
                self.diagnostics.source_warning(warning);
            }
            if let Some(message) = source.error.or(source.warning) {
                self.source_message = Some(message);
            }
        }
        if pending.overflow.is_some() && pending.overflow == self.session.id {
            self.message = Some(
                t(
                    "连接事件积压，已请求停止。",
                    "Connection events backed up; a stop was requested.",
                )
                .into(),
            );
            self.stop(cx);
        }
        for (id, event) in pending.controls {
            self.accept_stream(id, event, window, cx);
        }
        if let Some(id) = pending.playing {
            self.session.accept(id, StreamUpdate::Playing);
        }
        if let Some((id, event)) = pending.finished {
            self.accept_stream(id, event, window, cx);
            // 串流在后台结束：窗口不可见时停掉随之留下的预览采集。
            // The stream ended in the background: stop the preview capture it leaves behind.
            self.settle_capture(window, cx);
        }
        cx.notify();
    }

    fn accept_stream(
        &mut self,
        id: u64,
        event: StreamUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requests_password = matches!(event, StreamUpdate::Password(_));
        let finished = matches!(event, StreamUpdate::Finished { .. });
        let ready = matches!(event, StreamUpdate::Ready);
        let volume_report = match &event {
            StreamUpdate::Volume(report) => Some(*report),
            _ => None,
        };
        if !self.session.accept(id, event) {
            return;
        }
        if let Some(report) = volume_report {
            if report == VolumeReport::Rejected {
                self.message = Some(
                    t(
                        "接收端未接受音量更改。",
                        "The receiver did not accept the volume change.",
                    )
                    .into(),
                );
            }
            if self.volume_task.is_none() {
                self.volume_draft = None;
            }
        }
        if finished {
            self.volume_task = None;
            self.volume_draft = None;
        }
        if ready || finished {
            self.retry_secret = None;
            if self.session.prompt.is_none() {
                self.clear_password(cx);
            }
        }
        if (requests_password || finished)
            && let Some(host) = self.session.prompt.clone()
        {
            if let Some(secret) = self.retry_secret.take() {
                self.send_password(id, host, secret, cx);
            } else {
                if let Some(name) = self.prompt_name() {
                    self.password.update(cx, |input, cx| {
                        input.set_placeholder(
                            tf!(
                                "输入 {name} 的 AirPlay 密码",
                                "Enter the AirPlay password for {name}"
                            ),
                            cx,
                        );
                        // 密码错误后全选原输入，直接重新键入即可覆盖。
                        // After a rejection, select the old text so typing replaces it.
                        if finished {
                            let end = input.text().len();
                            input.select(0..end, cx);
                        }
                    });
                }
                let focus = self.password.read(cx).focus().clone();
                window.focus(&focus, cx);
            }
        }
    }

    // 音量 / volume

    /// 滑块显示的音量：拖动中的目标优先，其次是接收端确认值。
    /// Level the slider shows: the drag target first, else the receiver-confirmed level.
    pub(crate) fn volume_level(&self) -> f64 {
        self.volume_draft
            .or(self.session.volume.confirmed.map(f64::from))
            .unwrap_or(50.)
    }

    pub(crate) fn schedule_volume(
        &mut self,
        level: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.phase != Phase::Playing {
            return;
        }
        self.volume_draft = Some(level);
        // 拖动时保留最后一个目标，避免连续滑块事件塞满后端的音量通道。
        // Keep the latest drag target to avoid flooding the backend volume channel.
        self.volume_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = view.update(cx, |player, cx| player.send_volume(level as f32, cx));
        }));
        cx.notify();
    }

    pub(crate) fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.send_volume(self.session.volume.mute_target(), cx);
    }

    fn send_volume(&mut self, level: f32, cx: &mut Context<Self>) {
        self.volume_task = None;
        if self.session.phase != Phase::Playing || self.engine.is_quitting() {
            return;
        }
        self.message = None;
        if self.session.volume.confirmed == Some(level) {
            self.volume_draft = None;
        } else if let Err(error) = self.engine.set_volume(f64::from(level)) {
            self.message = Some(error);
            self.volume_draft = None;
        }
        cx.notify();
    }

    // 设置 / settings

    /// 保存语言选择并立即切换界面；保存失败时保持原语言。
    /// Saves the language choice and switches the UI at once; a failed save keeps the old one.
    pub(crate) fn set_language(&mut self, choice: lang::Choice, cx: &mut Context<Self>) {
        if choice == self.language {
            return;
        }
        match lang::save(self.engine.data_root(), choice) {
            Ok(()) => {
                self.language = choice;
                lang::set(choice);
                lang::sync_components(cx);
                self.password.update(cx, |input, cx| {
                    input.set_placeholder(t("输入 AirPlay 密码", "Enter the AirPlay password"), cx)
                });
            }
            Err(error) => self.message = Some(error),
        }
        cx.notify();
    }

    fn commit_settings(
        &mut self,
        next: Settings,
        job: Result<DesktopJob<()>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.message = None;
        let appearance_changed = next.appearance != self.settings.appearance;
        self.run_job(
            Op::SavingSettings,
            job,
            window,
            cx,
            move |player, result, window, cx| match result {
                Ok(()) => {
                    player.settings = next;
                    if appearance_changed {
                        player.apply_appearance(window, cx);
                    }
                }
                Err(error) => player.message = Some(error),
            },
        );
    }

    /// 后台确认后才更新已保存设置；失败时界面保持原值。
    /// Publish saved settings only after the worker confirms; on failure the view keeps the old ones.
    pub(crate) fn save_settings(
        &mut self,
        next: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_locked() {
            return;
        }
        let job = self.engine.prepare_save_settings(next.clone());
        self.commit_settings(next, job, window, cx);
    }

    /// 播放提前量与 Buffer 下次连接生效，串流占用期间不允许修改。
    /// Latency and buffer apply on the next connection and cannot change while a session holds them.
    pub(crate) fn save_stream_settings(
        &mut self,
        next: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let unchanged =
            next.latency == self.settings.latency && next.buffer == self.settings.buffer;
        if self.locked() || unchanged {
            return;
        }
        self.save_settings(next, window, cx);
    }

    pub(crate) fn apply_appearance(&self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.settings.appearance {
            Appearance::System => Mode::from(window.appearance()),
            Appearance::Light => Mode::Light,
            Appearance::Dark => Mode::Dark,
        };
        Theme::set_mode(mode, cx);
    }

    pub(crate) fn set_mapping(
        &mut self,
        side: usize,
        channel: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_locked() || self.settings.mapping[side] == channel {
            return;
        }
        let mut next = self.settings.clone();
        next.mapping[side] = channel;
        let job = self.engine.prepare_set_mapping(next.mapping);
        self.commit_settings(next, job, window, cx);
    }

    pub(crate) fn swap_speakers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_locked()
            || !self.connected()
            || !self
                .receiver()
                .is_some_and(|receiver| receiver.names.len() == 2)
        {
            return;
        }
        let next = Settings {
            speakers_swapped: !self.settings.speakers_swapped,
            ..self.settings.clone()
        };
        let job = self.engine.prepare_set_speaker_order(next.speakers_swapped);
        self.commit_settings(next, job, window, cx);
    }

    pub(crate) fn change_autostart(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.operation.is_some() || self.engine.is_quitting() {
            return;
        }
        self.message = None;
        self.run_job(
            Op::Autostart,
            Ok(Box::new(move || set_autostart(enabled))),
            window,
            cx,
            move |player, result, _, _| match result {
                Ok(()) => player.autostart = enabled,
                Err(error) => player.message = Some(error),
            },
        );
    }

    pub(crate) fn reset_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let Some(names) = self.receiver().map(|receiver| receiver.names.clone()) else {
            return;
        };
        self.message = None;
        let job = self.engine.prepare_forget_auth_policy(names);
        self.run_job(
            Op::ForgettingAuth,
            job,
            window,
            cx,
            |player, result, _, _| match result {
                Ok(()) => {
                    player.auth_notice = Some(t("已清除所选设备记录，下次连接重新检测。", "Cleared for the selected device; it will be detected again on the next connection.").into())
                }
                Err(error) => player.message = Some(error),
            },
        );
    }
}
