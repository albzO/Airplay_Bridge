//! 播放与常规设置控件。后台确认后才更新已保存设置。
//! Playback and general controls. Publish saved settings after worker confirmation.
use super::{Phase, Player};
use airplay_desktop::{Appearance, DesktopJob, Settings, set_autostart};
use gpui::{Context, IntoElement, Window, div, prelude::*};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
    slider::Slider,
    switch::Switch,
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Playback,
    Settings,
}

impl Player {
    fn settings_locked(&self) -> bool {
        self.operation.is_some()
            || self.engine.is_quitting()
            || (self.session.id.is_some()
                && !matches!(self.session.phase, Phase::Ready | Phase::Playing))
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
        let latency_changed = next.latency != self.settings.latency;
        let buffer_changed = next.buffer != self.settings.buffer;
        if latency_changed && job.is_err() {
            self.reset_latency_input(window, cx);
        }
        if buffer_changed && job.is_err() {
            self.reset_buffer_input(window, cx);
        }
        self.run_job(
            "正在保存设置",
            job,
            window,
            cx,
            move |player, result, window, cx| {
                match result {
                    Ok(()) => {
                        player.settings = next;
                        if appearance_changed {
                            player.apply_appearance(window, cx);
                        }
                    }
                    Err(error) => player.message = Some(error),
                }
                if latency_changed {
                    player.reset_latency_input(window, cx);
                }
                if buffer_changed {
                    player.reset_buffer_input(window, cx);
                }
            },
        );
    }

    pub(super) fn save_settings(
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

    pub(super) fn apply_appearance(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.settings.appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }

    pub(super) fn reset_latency_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.latency_input.update(cx, |input, cx| {
            input.set_value(self.settings.latency.to_string(), window, cx)
        });
    }

    pub(super) fn save_latency(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let value = self.latency_input.read(cx).value();
        let Ok(latency) = value.trim().parse::<u32>() else {
            self.message = Some("播放提前量应为 250–2000 ms 的整数。".into());
            self.reset_latency_input(window, cx);
            cx.notify();
            return;
        };
        if latency == self.settings.latency {
            self.reset_latency_input(window, cx);
            return;
        }
        self.save_settings(
            Settings {
                latency,
                ..self.settings.clone()
            },
            window,
            cx,
        );
    }

    fn set_mapping(
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

    fn swap_speakers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_locked()
            || !matches!(self.session.phase, Phase::Ready | Phase::Playing)
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

    fn change_autostart(&mut self, enabled: bool, window: &Window, cx: &mut Context<Self>) {
        if self.operation.is_some() || self.engine.is_quitting() {
            return;
        }
        self.message = None;
        self.run_job(
            "正在更新开机自启",
            Ok(Box::new(move || set_autostart(enabled))),
            window,
            cx,
            move |player, result, _, _| match result {
                Ok(()) => player.autostart = enabled,
                Err(error) => player.message = Some(error),
            },
        );
    }

    fn toggle_capture(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.locked() || self.source().is_none() {
            return;
        }
        self.auto_capture_pending = false;
        self.source_message = None;
        self.message = None;
        let enabled = !self.capture_enabled;
        let job = self
            .engine
            .prepare_set_capture_enabled(self.inbox.sink(), enabled);
        self.run_job(
            "正在切换采集",
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

    pub(super) fn sync_volume(&self, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.session.volume.confirmed.unwrap_or(50.);
        self.volume_slider
            .update(cx, |slider, cx| slider.set_value(level, window, cx));
    }

    pub(super) fn schedule_volume(&mut self, level: f32, window: &Window, cx: &mut Context<Self>) {
        if self.session.phase != Phase::Playing {
            return;
        }
        // 拖动时保留最后一个目标，避免连续滑块事件塞满后端的音量通道。
        // Keep the latest drag target to avoid flooding the backend volume channel.
        self.volume_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = cx.update(|window, cx| {
                view.update(cx, |player, cx| player.send_volume(level, window, cx))
            });
        }));
    }

    fn send_volume(&mut self, level: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.volume_task = None;
        if self.session.phase != Phase::Playing || self.engine.is_quitting() {
            return;
        }
        self.message = None;
        if self.session.volume.confirmed == Some(level) {
            self.sync_volume(window, cx);
        } else if let Err(error) = self.engine.set_volume(f64::from(level)) {
            self.message = Some(error);
            self.sync_volume(window, cx);
        }
        cx.notify();
    }

    pub(super) fn volume_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let enabled = self.session.phase == Phase::Playing && !self.engine.is_quitting();
        let level = self.session.volume.confirmed;
        let pair = self
            .receiver()
            .filter(|receiver| receiver.names.len() == 2)
            .filter(|_| matches!(self.session.phase, Phase::Ready | Phase::Playing));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("mute")
                            .label(if level == Some(0.) {
                                "取消静音"
                            } else {
                                "静音"
                            })
                            .disabled(!enabled)
                            .on_click(cx.listener(|player, _, window, cx| {
                                player.send_volume(player.session.volume.mute_target(), window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Slider::new(&self.volume_slider).disabled(!enabled)),
                    )
                    .child(
                        div().text_sm().child(
                            level.map_or_else(|| "—".into(), |value| format!("{value:.0}%")),
                        ),
                    ),
            )
            .when_some(pair, |el, receiver| {
                let left = usize::from(self.settings.speakers_swapped);
                el.child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .flex_wrap()
                        .child(
                            div()
                                .text_sm()
                                .child(format!("左 · {}", receiver.names[left])),
                        )
                        .child(
                            Button::new("swap-speakers")
                                .label("交换左右")
                                .disabled(self.settings_locked())
                                .on_click(cx.listener(|player, _, window, cx| {
                                    player.swap_speakers(window, cx)
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .child(format!("右 · {}", receiver.names[1 - left])),
                        ),
                )
            })
    }

    pub(super) fn capture_button(&self, cx: &Context<Self>) -> Button {
        Button::new("capture-toggle")
            .label(if self.capture_enabled {
                "关闭采集"
            } else {
                "开启采集"
            })
            .disabled(self.locked() || self.source().is_none())
            .on_click(cx.listener(|player, _, window, cx| player.toggle_capture(window, cx)))
    }

    pub(super) fn source_menu(&self, label: String, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        Button::new("source-menu")
            .label(format!("{label} ▾"))
            .disabled(self.locked() || self.inputs.is_empty())
            .dropdown_menu(move |mut menu, _, cx| {
                let player = view.read(cx);
                for (playback, title) in [(true, "播放设备"), (false, "录音设备")] {
                    menu = menu.label(title);
                    for (index, input) in player
                        .inputs
                        .iter()
                        .enumerate()
                        .filter(|(_, input)| (input.flow == "playback") == playback)
                    {
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(input.name.clone())
                                .disabled(input.channels.unwrap_or(0) == 0)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |player, cx| {
                                        player.select_source(index, window, cx)
                                    })
                                }),
                        );
                    }
                }
                menu
            })
    }

    fn mapping_menu(&self, side: usize, cx: &Context<Self>) -> impl IntoElement {
        let channels = self
            .source()
            .and_then(|source| source.channels)
            .unwrap_or(0) as usize;
        let view = cx.entity();
        Button::new(("mapping", side))
            .label(format!("输入声道 {} ▾", self.settings.mapping[side] + 1))
            .disabled(channels == 0 || self.settings_locked())
            .dropdown_menu(move |mut menu, _, _| {
                for channel in 0..channels {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("输入声道 {}", channel + 1)).on_click(
                            move |_, window, cx| {
                                view.update(cx, |player, cx| {
                                    player.set_mapping(side, channel, window, cx)
                                })
                            },
                        ),
                    );
                }
                menu
            })
    }

    pub(super) fn general_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let disabled = self.settings_locked();
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        div()
            .id("general-settings")
            .flex_1()
            .min_h_0()
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
                    .child(div().font_semibold().child("应用行为"))
                    .child(setting_row(
                        "开机自启",
                        Switch::new("autostart")
                            .checked(self.autostart)
                            .disabled(self.operation.is_some() || self.engine.is_quitting())
                            .on_click(cx.listener(|player, enabled, window, cx| {
                                player.change_autostart(*enabled, window, cx)
                            })),
                    ))
                    .child(setting_row(
                        "保持系统唤醒",
                        Switch::new("keep-awake")
                            .checked(self.settings.keep_awake)
                            .disabled(disabled)
                            .on_click(cx.listener(|player, enabled, window, cx| {
                                player.save_settings(
                                    Settings {
                                        keep_awake: *enabled,
                                        ..player.settings.clone()
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    ))
                    .child(setting_row(
                        "关闭窗口时",
                        div().flex().gap_2().children(
                            [("tray", "收起到托盘"), ("quit", "退出应用")]
                                .into_iter()
                                .map(|(value, label)| {
                                    choice(
                                        Button::new(value).label(label),
                                        self.settings.close_action == value,
                                    )
                                    .disabled(disabled || self.settings.close_action == value)
                                    .on_click(cx.listener(
                                        move |player, _, window, cx| {
                                            player.save_settings(
                                                Settings {
                                                    close_action: value.into(),
                                                    ..player.settings.clone()
                                                },
                                                window,
                                                cx,
                                            );
                                        },
                                    ))
                                }),
                        ),
                    ))
                    .child(setting_row(
                        "外观",
                        div().flex().gap_2().children(
                            [
                                (Appearance::System, "跟随系统"),
                                (Appearance::Light, "浅色"),
                                (Appearance::Dark, "深色"),
                            ]
                            .into_iter()
                            .enumerate()
                            .map(|(index, (appearance, label))| {
                                choice(
                                    Button::new(("appearance", index)).label(label),
                                    self.settings.appearance == appearance,
                                )
                                .disabled(disabled || self.settings.appearance == appearance)
                                .on_click(cx.listener(
                                    move |player, _, window, cx| {
                                        player.save_settings(
                                            Settings {
                                                appearance,
                                                ..player.settings.clone()
                                            },
                                            window,
                                            cx,
                                        );
                                    },
                                ))
                            }),
                        ),
                    ))
                    .child(
                        div()
                            .border_t_1()
                            .border_color(border)
                            .pt_4()
                            .font_semibold()
                            .child("播放设置"),
                    )
                    .child(setting_row(
                        "播放提前量",
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w_24()
                                    .child(Input::new(&self.latency_input).disabled(self.locked())),
                            )
                            .child(div().text_sm().text_color(muted).child("ms"))
                            .child(
                                Button::new("latency-save")
                                    .label("保存")
                                    .disabled(self.locked())
                                    .on_click(cx.listener(|player, _, window, cx| {
                                        player.save_latency(window, cx)
                                    })),
                            ),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("250–2000 ms，下次连接生效。"),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(border)
                            .pt_4()
                            .font_semibold()
                            .child("输入声道映射"),
                    )
                    .child(setting_row("左输出取样", self.mapping_menu(0, cx)))
                    .child(setting_row("右输出取样", self.mapping_menu(1, cx))),
            )
    }
}

fn setting_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(label)
        .child(control)
}

fn choice(button: Button, selected: bool) -> Button {
    if selected {
        button.primary()
    } else {
        button.outline()
    }
}
