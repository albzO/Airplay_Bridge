//! GPUI 播放页：来源、接收端、会话和密码交互。
//! GPUI player: source, receiver, session and password controls.
mod controls;

use crate::{
    inbox::{Inbox, Pending, StreamUpdate},
    session::{Phase, Session},
    volume::VolumeReport,
};
use airplay_desktop::{Appearance, DesktopJob, Engine, Initialization, Settings};
use controls::Page;
use gpui::{Context, Entity, Focusable, IntoElement, Render, Task, Window, div, prelude::*, rgb};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Copy as CopyText, Cut, Input as TextInput, InputEvent, InputState, SelectAll},
    slider::{SliderEvent, SliderState, SliderValue},
};
use homepod_test::{capture::Input, discovery::Device};
use std::{collections::BTreeMap, sync::Arc};

struct Receiver {
    key: String,
    label: String,
    names: Vec<String>,
}

fn receivers(devices: &[Device]) -> Vec<Receiver> {
    let mut groups = BTreeMap::<&str, Vec<&Device>>::new();
    for device in devices {
        if let Some(group) = device.properties.get("tsid").filter(|id| !id.is_empty()) {
            groups.entry(group).or_default().push(device);
        }
    }
    let mut list = Vec::new();
    for (group, mut members) in groups.into_iter().filter(|(_, members)| members.len() == 2) {
        members.sort_by_key(|d| {
            (
                d.properties.get("igl").map(String::as_str) != Some("1"),
                &d.service,
            )
        });
        let names: Vec<_> = members.into_iter().map(|d| d.name.clone()).collect();
        list.push(Receiver {
            key: format!("pair:{group}"),
            label: names.join(" + "),
            names,
        });
    }
    list.extend(devices.iter().map(|d| Receiver {
        key: format!("device:{}", d.service),
        label: d.name.clone(),
        names: vec![d.name.clone()],
    }));
    list
}

pub struct Player {
    engine: Arc<Engine>,
    inbox: Inbox,
    settings: Settings,
    inputs: Vec<Input>,
    devices: Vec<Device>,
    receivers: Vec<Receiver>,
    selected: Option<String>,
    operation: Option<&'static str>,
    backend_available: bool,
    capture_enabled: bool,
    auto_capture_pending: bool,
    page: Page,
    autostart: bool,
    latency_input: Entity<InputState>,
    volume_slider: Entity<SliderState>,
    volume_task: Option<Task<()>>,
    peaks: [f32; 2],
    message: Option<String>,
    source_message: Option<String>,
    session: Session,
    password: Entity<InputState>,
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
        let password = Self::password_input(window, cx);
        let latency_input = cx.new(|cx| InputState::new(window, cx).default_value("300"));
        cx.subscribe_in(&latency_input, window, |player, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { secondary: false }) {
                player.save_latency(window, cx);
            }
        })
        .detach();
        let volume_slider = cx.new(|_| SliderState::new().max(100.).step(1.).default_value(50.));
        cx.subscribe_in(&volume_slider, window, |player, _, event, window, cx| {
            if let SliderEvent::Change(SliderValue::Single(level)) = *event {
                player.schedule_volume(level, window, cx);
            }
        })
        .detach();
        let view = cx.weak_entity();
        window
            .observe_window_appearance(move |window, cx| {
                let _ = view.update(cx, |player, cx| {
                    if player.settings.appearance == Appearance::System {
                        player.apply_appearance(window, cx);
                    }
                });
            })
            .detach();
        cx.on_next_frame(window, |player, window, cx| player.initialize(window, cx));
        Self {
            engine,
            inbox,
            settings: Settings::default(),
            inputs: Vec::new(),
            devices: Vec::new(),
            receivers: Vec::new(),
            selected: None,
            operation: Some("正在初始化"),
            backend_available: false,
            capture_enabled: false,
            auto_capture_pending: true,
            page: Page::Playback,
            autostart: false,
            latency_input,
            volume_slider,
            volume_task: None,
            peaks: [0.; 2],
            message: None,
            source_message: None,
            session: Session::default(),
            password,
            retry_secret: None,
        }
    }

    fn password_input(window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("输入 AirPlay 密码")
        });
        cx.subscribe_in(&password, window, |player, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { secondary: false }) {
                player.submit_password(window, cx);
            }
        })
        .detach();
        password
    }

    fn run_job<T: Send + 'static>(
        &mut self,
        label: &'static str,
        job: Result<DesktopJob<T>, String>,
        window: &Window,
        cx: &mut Context<Self>,
        complete: impl FnOnce(&mut Self, Result<T, String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let job = match job {
            Ok(job) => job,
            Err(error) => {
                self.message = Some(error);
                cx.notify();
                return;
            }
        };
        self.operation = Some(label);
        cx.notify();
        cx.spawn_in(window, async move |view, cx| {
            let result = cx.background_executor().spawn(async move { job() }).await;
            let _ = cx.update(|window, cx| {
                view.update(cx, |player, cx| {
                    player.operation = None;
                    complete(player, result, window, cx);
                    cx.notify();
                })
            });
        })
        .detach();
    }

    fn initialize(&mut self, window: &Window, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        self.run_job(
            "正在读取音频设备",
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
        self.message = state.awake_error.or(state.autostart_error);
        self.reset_latency_input(window, cx);
        self.apply_appearance(window, cx);
        self.set_devices(state.devices);
        self.selected = self.receivers.first().map(|receiver| receiver.key.clone());
        if !self.backend_available {
            self.message = Some("缺少原生后端，请从完整的 GPUI 便携目录启动。".into());
        }
        if self.source().is_some() {
            self.auto_capture_pending = false;
            self.preview(window, cx, self.backend_available);
        } else if self.backend_available {
            self.refresh(window, cx);
        }
    }

    fn source(&self) -> Option<&Input> {
        self.inputs
            .iter()
            .find(|source| source.id == self.settings.endpoint && source.channels.unwrap_or(0) > 0)
    }

    fn receiver(&self) -> Option<&Receiver> {
        let key = self.selected.as_ref()?;
        self.receivers.iter().find(|receiver| &receiver.key == key)
    }

    fn set_devices(&mut self, devices: Vec<Device>) {
        self.receivers = receivers(&devices);
        self.devices = devices;
        if self.receiver().is_none() {
            self.selected = None;
            self.session = Session::default();
        }
    }

    fn locked(&self) -> bool {
        self.operation.is_some() || self.session.id.is_some() || self.engine.is_quitting()
    }

    fn refresh(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let job = self.engine.prepare_discover_devices();
        self.run_job(
            "正在发现接收端",
            job,
            window,
            cx,
            |player, result, _, _| match result {
                Ok(devices) => player.set_devices(devices),
                Err(error) => player.message = Some(error),
            },
        );
    }

    fn select_source(&mut self, index: usize, window: &Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let source = &self.inputs[index];
        let mut next = self.settings.clone();
        next.endpoint = source.id.clone();
        next.mapping = [0, usize::from(source.channels.unwrap_or(0) > 1)];
        self.message = None;
        let job = self.engine.prepare_save_settings(next.clone());
        self.run_job(
            "正在保存来源",
            job,
            window,
            cx,
            move |player, result, window, cx| match result {
                Ok(()) => {
                    player.settings = next;
                    player.peaks = [0.; 2];
                    player.source_message = None;
                    if player.capture_enabled || player.auto_capture_pending {
                        player.auto_capture_pending = false;
                        player.preview(window, cx, false);
                    }
                }
                Err(error) => player.message = Some(error),
            },
        );
    }

    fn preview(&mut self, window: &Window, cx: &mut Context<Self>, discover_after: bool) {
        let sink = self.inbox.sink();
        let job = if self.capture_enabled {
            self.engine.prepare_monitor_source(sink)
        } else {
            self.engine.prepare_set_capture_enabled(sink, true)
        };
        self.run_job(
            "正在准备电平预览",
            job,
            window,
            cx,
            move |player, result, window, cx| {
                match result {
                    Ok(()) => player.capture_enabled = true,
                    Err(error) => player.source_message = Some(error),
                }
                if discover_after {
                    player.refresh(window, cx);
                }
            },
        );
    }

    fn start(&mut self, retry_secret: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() || self.source().is_none() || !self.backend_available {
            return;
        }
        let Some(names) = self.receiver().map(|receiver| receiver.names.clone()) else {
            return;
        };
        self.message = None;
        // start_stream 同步返回预留编号。GPUI 下一次处理队列前编号已登记。
        // start_stream returns the reservation synchronously, before GPUI drains the inbox.
        match self.engine.start_stream(
            self.inbox.sink(),
            names,
            self.settings.clone(),
            retry_secret.is_some(),
        ) {
            Ok(id) => {
                self.session.start(id);
                self.volume_task = None;
                self.sync_volume(window, cx);
                if retry_secret.is_none() {
                    self.clear_password(window, cx);
                }
                self.retry_secret = retry_secret;
            }
            Err(error) => self.message = Some(error),
        }
        cx.notify();
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.session.id {
            self.session.stop();
            self.volume_task = None;
            self.sync_volume(window, cx);
            self.retry_secret = None;
            self.clear_password(window, cx);
            self.engine.stop_stream(id);
            cx.notify();
        }
    }

    fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.session.prompt.clone() else {
            return;
        };
        let secret = self.password.read(cx).value().to_string();
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
            Ok(()) => {
                self.session.password_sent(host);
            }
            Err(error) => self.message = Some(error),
        }
        cx.notify();
    }

    fn clear_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password = Self::password_input(window, cx);
    }

    fn accept(&mut self, pending: Pending, window: &mut Window, cx: &mut Context<Self>) {
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
            if let Some(message) = source.error.or(source.warning) {
                self.source_message = Some(message);
            }
        }
        if pending.overflow.is_some() && pending.overflow == self.session.id {
            self.message = Some("连接事件积压，已请求停止。".into());
            self.stop(window, cx);
        }
        for (id, event) in pending.controls {
            self.accept_stream(id, event, window, cx);
        }
        if let Some(id) = pending.playing {
            self.session.accept(id, StreamUpdate::Playing);
        }
        if let Some((id, event)) = pending.finished {
            self.accept_stream(id, event, window, cx);
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
                self.message = Some("接收端未接受音量更改。".into());
            }
            if self.volume_task.is_none() {
                self.sync_volume(window, cx);
            }
        }
        if finished {
            self.volume_task = None;
            self.sync_volume(window, cx);
        }
        if ready || finished {
            self.retry_secret = None;
            if self.session.prompt.is_none() {
                self.clear_password(window, cx);
            }
        }
        if (requests_password || finished)
            && let Some(host) = self.session.prompt.clone()
        {
            if let Some(secret) = self.retry_secret.take() {
                self.send_password(id, host, secret, cx);
            } else {
                self.password
                    .update(cx, |input, cx| input.focus(window, cx));
                if finished {
                    let focus = self.password.read(cx).focus_handle(cx);
                    // 先绘制重新打开的输入框，再派发全选给它的焦点节点。
                    // Let the reopened field render before dispatching select-all to its focus node.
                    window.on_next_frame(move |window, _| {
                        window.on_next_frame(move |window, cx| {
                            if focus.is_focused(window) {
                                window.dispatch_action(Box::new(SelectAll), cx);
                            }
                        });
                    });
                }
            }
        }
    }

    fn source_group(&self, playback: bool, cx: &Context<Self>) -> impl IntoElement {
        let title = if playback {
            "播放设备"
        } else {
            "录音设备"
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(title),
            )
            .children(
                self.inputs
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| (i.flow == "playback") == playback)
                    .map(|(index, input)| {
                        let button = Button::new(("source", index))
                            .label(input.name.clone())
                            .w_full();
                        let button = if self.settings.endpoint == input.id {
                            button.primary()
                        } else {
                            button.ghost()
                        };
                        button
                            .disabled(self.locked() || input.channels.unwrap_or(0) == 0)
                            .on_click(cx.listener(move |player, _, window, cx| {
                                player.select_source(index, window, cx)
                            }))
                    }),
            )
    }
}

impl Render for Player {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let busy = self.session.id.is_some();
        let source_name = self
            .source()
            .map_or("请选择音频流来源", |source| source.name.as_str())
            .to_owned();
        let status = self.operation.unwrap_or(self.session.phase.label());
        let prompt = self.session.prompt.as_ref().map(|host| {
            self.devices
                .iter()
                .find(|device| {
                    device
                        .addresses
                        .iter()
                        .any(|address| address.to_string() == *host)
                })
                .map_or("当前接收端", |device| device.name.as_str())
                .to_owned()
        });
        let error = self
            .message
            .as_deref()
            .or(self.session.error.as_deref())
            .or(self.source_message.as_deref())
            .map(str::to_owned);
        div()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_2xl().font_semibold().child("AirPlay Bridge"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child("将 Windows 音频发送到 AirPlay 接收端"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_sm().text_color(muted).child(status))
                            .child(
                                Button::new("page")
                                    .label(if self.page == Page::Playback {
                                        "设置"
                                    } else {
                                        "返回播放"
                                    })
                                    .on_click(cx.listener(|player, _, _, cx| {
                                        player.page = if player.page == Page::Playback {
                                            Page::Settings
                                        } else {
                                            Page::Playback
                                        };
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(self.page == Page::Playback, |root| {
                root.child(
                    div()
                        .flex()
                        .gap_4()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .id("sources")
                                .flex_1()
                                .overflow_y_scroll()
                                .border_1()
                                .border_color(border)
                                .rounded_lg()
                                .p_4()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        .child(div().font_semibold().child("音频来源"))
                                        .child(self.source_group(true, cx))
                                        .child(self.source_group(false, cx)),
                                ),
                        )
                        .child(
                            div()
                                .id("receivers")
                                .flex_1()
                                .overflow_y_scroll()
                                .border_1()
                                .border_color(border)
                                .rounded_lg()
                                .p_4()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div().font_semibold().child("AirPlay 接收端"),
                                                )
                                                .child(
                                                    Button::new("refresh")
                                                        .label("刷新")
                                                        .disabled(
                                                            self.locked()
                                                                || !self.backend_available,
                                                        )
                                                        .on_click(cx.listener(
                                                            |player, _, window, cx| {
                                                                player.message = None;
                                                                player.refresh(window, cx);
                                                            },
                                                        )),
                                                ),
                                        )
                                        .child(self.volume_controls(cx))
                                        .when(self.receivers.is_empty(), |el| {
                                            el.child(
                                                div()
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .child("尚未发现接收端"),
                                            )
                                        })
                                        .children(self.receivers.iter().enumerate().map(
                                            |(index, receiver)| {
                                                let key = receiver.key.clone();
                                                let button = Button::new(("receiver", index))
                                                    .label(receiver.label.clone())
                                                    .w_full();
                                                let button = if self.selected.as_ref() == Some(&key)
                                                {
                                                    button.primary()
                                                } else {
                                                    button.ghost()
                                                };
                                                button.disabled(self.locked()).on_click(
                                                    cx.listener(move |player, _, window, cx| {
                                                        if player.selected.as_ref() != Some(&key) {
                                                            player.selected = Some(key.clone());
                                                            player.session = Session::default();
                                                            player.message = None;
                                                            player.clear_password(window, cx);
                                                            cx.notify();
                                                        }
                                                    }),
                                                )
                                            },
                                        )),
                                ),
                        ),
                )
            })
            .when(self.page == Page::Settings, |root| {
                root.child(self.settings_page(cx))
            })
            .when_some(prompt, |el, name| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .child(format!("输入 {name} 的 AirPlay 密码")),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .capture_action(|_: &CopyText, _, cx| cx.stop_propagation())
                                        .capture_action(|_: &Cut, _, cx| cx.stop_propagation())
                                        .child(TextInput::new(&self.password)),
                                )
                                .child(
                                    Button::new("password-submit")
                                        .label("确认")
                                        .primary()
                                        .on_click(cx.listener(|player, _, window, cx| {
                                            player.submit_password(window, cx)
                                        })),
                                ),
                        ),
                )
            })
            .when_some(error, |el, message| {
                el.child(div().text_sm().text_color(cx.theme().danger).child(message))
            })
            .child(
                div()
                    .flex()
                    .gap_4()
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .pt_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_2()
                            .child(self.source_menu(source_name, cx))
                            .child(div().flex().gap_2().children(self.peaks.map(|peak| {
                                let fill = if peak > 0. {
                                    ((20. * peak.log10() + 60.) / 60.).clamp(0., 1.)
                                } else {
                                    0.
                                };
                                div().flex_1().h_1().rounded_full().bg(border).child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(fill))
                                        .rounded_full()
                                        .bg(rgb(0x2aaa73)),
                                )
                            }))),
                    )
                    .child(self.capture_button(cx))
                    .child(if busy {
                        Button::new("stop")
                            .label("停止串流")
                            .danger()
                            .disabled(self.session.phase == Phase::Stopping)
                            .on_click(cx.listener(|player, _, window, cx| player.stop(window, cx)))
                    } else {
                        Button::new("start")
                            .label("开始串流")
                            .primary()
                            .disabled(
                                self.locked()
                                    || !self.backend_available
                                    || self.source().is_none()
                                    || self.receiver().is_none(),
                            )
                            .on_click(
                                cx.listener(|player, _, window, cx| player.start(None, window, cx)),
                            )
                    }),
            )
    }
}
