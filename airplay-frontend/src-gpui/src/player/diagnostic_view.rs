//! 设置页中的运行统计、技术详情与日志。
//! Runtime statistics, technical details, and logs in Settings.
use super::Player;
use airplay_desktop::Settings;
use gpui::{
    ClipboardItem, Context, Div, IntoElement, ListHorizontalSizingBehavior, Window, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    switch::Switch,
};
use homepod_test::capture::Format;
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    General,
    Runtime,
    Technical,
    Logs,
}

#[derive(Clone, Copy)]
enum Lines {
    Logs,
    Capture,
    Report,
}

impl Player {
    pub(super) fn settings_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let tabs = [
            (SettingsTab::General, "常规"),
            (SettingsTab::Runtime, "运行统计"),
            (SettingsTab::Technical, "技术详情"),
            (SettingsTab::Logs, "日志"),
        ];
        let body = match self.settings_tab {
            SettingsTab::General => self.general_settings(cx).into_any_element(),
            SettingsTab::Runtime => self.runtime_settings(cx).into_any_element(),
            SettingsTab::Technical => self.technical_settings(cx).into_any_element(),
            SettingsTab::Logs => self.log_settings(cx).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .children(tabs.into_iter().enumerate().map(|(index, (tab, label))| {
                        let button = Button::new(("settings-tab", index)).label(label);
                        let button = if self.settings_tab == tab {
                            button.primary()
                        } else {
                            button.ghost()
                        };
                        button.on_click(cx.listener(move |player, _, _, cx| {
                            player.settings_tab = tab;
                            cx.notify();
                        }))
                    })),
            )
            .child(body)
    }

    fn runtime_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = &self.diagnostics.telemetry;
        let report = self.diagnostics.report.as_ref();
        let caption = cx.theme().muted_foreground;
        let counters = [
            "设备",
            "发送包",
            "本地丢包",
            "同步丢包",
            "重传请求",
            "已重传",
            "过期",
        ];
        scroll_page("runtime-stats")
            .child(
                div().flex().gap_3().children(
                    [
                        ("运行时长", format!("{} s", decimal(t.elapsed_seconds, 0))),
                        ("流水线水位", format!("{} ms", decimal(t.water_ms, 1))),
                        (
                            "漂移校正",
                            format!("{} ppm", decimal(t.controller.correction_ppm, 1)),
                        ),
                    ]
                    .into_iter()
                    .map(|(title, value)| {
                        panel(cx)
                            .flex_1()
                            .child(div().text_sm().text_color(caption).child(title))
                            .child(div().text_xl().font_semibold().child(value))
                    }),
                ),
            )
            .child(
                panel(cx)
                    .child(heading("音频流水线"))
                    .child(metric("采集帧数", count(t.capture_frames)))
                    .child(metric("输出帧数", count(t.output_frames)))
                    .child(metric(
                        "协议提前量",
                        format!("{} ms", decimal(t.lead_ms, 1)),
                    ))
                    .child(metric(
                        "估计发送端延迟",
                        format!(
                            "{} ms",
                            decimal(
                                t.lead_ms.zip(t.water_ms).map(|(lead, water)| lead + water),
                                1
                            )
                        ),
                    ))
                    .child(metric(
                        "采集不连续 / 时间戳错误",
                        report.map_or_else(
                            || "停止后汇总".into(),
                            |r| {
                                format!(
                                    "{} / {}",
                                    count(r.discontinuities),
                                    count(r.timestamp_errors)
                                )
                            },
                        ),
                    ))
                    .child(metric(
                        "转换汇总",
                        report
                            .and_then(|r| r.conversion.clone())
                            .unwrap_or_else(|| "停止后汇总".into()),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(caption)
                            .child("延迟估计不含应用和扬声器的实际发声耗时。"),
                    ),
            )
            .child(
                panel(cx)
                    .child(heading("设备传输"))
                    .child(stat_row(counters.map(str::to_owned)).text_color(caption))
                    .when(self.diagnostics.devices.is_empty(), |el| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(caption)
                                .child("等待接收端传输统计。"),
                        )
                    })
                    .children(self.diagnostics.devices.iter().map(|device| {
                        let [sent, dropped, sync, requested, resent, expired] =
                            device.totals.map(count);
                        stat_row([
                            device.name.clone(),
                            sent,
                            dropped,
                            sync,
                            requested,
                            resent,
                            expired,
                        ])
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(caption)
                            .child("累计本次运行中最近 128 个设备的统计，退出软件后清空。"),
                    ),
            )
    }

    pub(super) fn reset_buffer_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.buffer_input.update(cx, |input, cx| {
            input.set_value(self.settings.buffer.to_string(), window, cx)
        });
    }

    pub(super) fn save_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let value = self.buffer_input.read(cx).value();
        let Ok(buffer) = value.trim().parse::<u32>() else {
            self.message = Some("Buffer 应为 64–512 ms 的整数。".into());
            self.reset_buffer_input(window, cx);
            cx.notify();
            return;
        };
        if buffer == self.settings.buffer {
            self.reset_buffer_input(window, cx);
            return;
        }
        self.save_settings(
            Settings {
                buffer,
                ..self.settings.clone()
            },
            window,
            cx,
        );
    }

    fn reset_auth(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let Some(names) = self.receiver().map(|receiver| receiver.names.clone()) else {
            return;
        };
        self.message = None;
        let job = self.engine.prepare_forget_auth_policy(names);
        self.run_job(
            "正在清除密码要求记录",
            job,
            window,
            cx,
            |player, result, _, _| match result {
                Ok(()) => {
                    player.auth_notice = Some("已清除所选设备记录，下次连接重新检测。".into())
                }
                Err(error) => player.message = Some(error),
            },
        );
    }

    fn technical_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let source = self.source();
        let muted = cx.theme().muted_foreground;
        let details = &self.diagnostics;
        scroll_page("technical-details")
            .child(
                panel(cx)
                    .child(heading("采集与处理"))
                    .child(metric(
                        "设备格式",
                        audio_format(source.and_then(|input| input.device_format.as_ref())),
                    ))
                    .child(metric(
                        "WASAPI 采集格式",
                        audio_format(source.and_then(|input| input.mix_format.as_ref())),
                    ))
                    .child(metric(
                        "本次采集采样率",
                        details
                            .telemetry
                            .input_rate
                            .map_or_else(|| "—".into(), |rate| format!("{rate} Hz")),
                    ))
                    .child(metric("发送格式", "44100 Hz · 16-bit PCM → ALAC".into()))
                    .child(metric(
                        "左右电平",
                        format!("{} / {}", level(self.peaks[0]), level(self.peaks[1])),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child("Buffer")
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().w_24().child(
                                        Input::new(&self.buffer_input).disabled(self.locked()),
                                    ))
                                    .child(div().text_sm().text_color(muted).child("ms"))
                                    .child(
                                        Button::new("save-buffer")
                                            .label("保存")
                                            .disabled(self.locked())
                                            .on_click(cx.listener(|player, _, window, cx| {
                                                player.save_buffer(window, cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("64–512 ms，下次连接生效。"),
                    ),
            )
            .child(
                panel(cx)
                    .child(heading("会话协议"))
                    .child(metric(
                        "时钟协议",
                        details.clock.clone().unwrap_or_else(|| "—".into()),
                    ))
                    .child(metric(
                        "认证方式",
                        details.auth.clone().unwrap_or_else(|| "—".into()),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child("密码要求记录")
                            .child(
                                Button::new("reset-auth")
                                    .label("重新检测所选设备")
                                    .disabled(self.locked() || self.receiver().is_none())
                                    .on_click(cx.listener(|player, _, window, cx| {
                                        player.reset_auth(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div().text_sm().text_color(muted).child(
                            self.auth_notice.clone().unwrap_or_else(|| {
                                "只记住密码要求。取消设备密码后可重新检测。".into()
                            }),
                        ),
                    ),
            )
            .child(
                panel(cx)
                    .child(heading("设备能力"))
                    .when(self.devices.is_empty(), |el| {
                        el.child(div().text_sm().text_color(muted).child("尚未发现接收端。"))
                    })
                    .children(self.devices.iter().enumerate().map(|(index, device)| {
                        let key = device.service.clone();
                        let open = self.device_details.as_ref() == Some(&key);
                        let addresses = device
                            .addresses
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ");
                        let label = format!(
                            "{} · {}:{} {}",
                            device.name,
                            addresses,
                            device.port,
                            if open { "▴" } else { "▾" }
                        );
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                Button::new(("device-capabilities", index))
                                    .label(label)
                                    .ghost()
                                    .on_click(cx.listener(move |player, _, _, cx| {
                                        player.device_details =
                                            if player.device_details.as_ref() == Some(&key) {
                                                None
                                            } else {
                                                Some(key.clone())
                                            };
                                        cx.notify();
                                    })),
                            )
                            .when(open, |el| {
                                el.children(
                                    device
                                        .properties
                                        .iter()
                                        .map(|(field, value)| metric(field.clone(), value.clone())),
                                )
                            })
                    })),
            )
    }

    fn log_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let details = &self.diagnostics;
        let muted = cx.theme().muted_foreground;
        scroll_page("session-logs")
            .child(
                panel(cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(heading("会话日志"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(self.copy_lines_button(
                                        "copy-log",
                                        "复制日志",
                                        Lines::Logs,
                                        cx,
                                    ))
                                    .child(
                                        Button::new("open-logs").label("打开日志目录").on_click(
                                            cx.listener(|player, _, _, cx| {
                                                if let Err(error) = player.engine.open_logs() {
                                                    player.message = Some(error);
                                                    cx.notify();
                                                }
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        Switch::new("detailed-logs")
                            .label("保存详细日志")
                            .checked(self.settings.detailed_logs)
                            .disabled(self.locked())
                            .on_click(cx.listener(|player, enabled, window, cx| {
                                player.save_settings(
                                    Settings {
                                        detailed_logs: *enabled,
                                        ..player.settings.clone()
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("下次连接生效。详细模式保存协议日志和流水线进度。"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(details.log_path.clone()),
                    )
                    .when_some(details.pipeline_path.clone(), |el, path| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(format!("流水线：{path}")),
                        )
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(if details.detailed {
                                "显示最近 300 条详细输出。"
                            } else {
                                "显示最近 300 条关键故障和告警。"
                            }),
                    )
                    .when(details.logs.is_empty(), |el| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(if details.detailed {
                                    "等待详细日志输出。"
                                } else {
                                    "暂无关键故障。"
                                }),
                        )
                    })
                    .when(!details.logs.is_empty(), |el| {
                        el.child(self.log_lines(Lines::Logs, cx))
                    })
                    .when(details.report.is_some(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Button::new("report-toggle")
                                        .label(if self.report_open {
                                            "收起会话报告"
                                        } else {
                                            "展开会话报告"
                                        })
                                        .on_click(cx.listener(|player, _, _, cx| {
                                            player.report_open = !player.report_open;
                                            cx.notify();
                                        })),
                                )
                                .child(self.copy_lines_button(
                                    "copy-report",
                                    "复制报告",
                                    Lines::Report,
                                    cx,
                                )),
                        )
                    })
                    .when(self.report_open && details.report.is_some(), |el| {
                        el.child(self.log_lines(Lines::Report, cx))
                    }),
            )
            .child(
                panel(cx)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(heading("采集逐包诊断"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(self.copy_lines_button(
                                        "copy-capture",
                                        "复制摘要",
                                        Lines::Capture,
                                        cx,
                                    ))
                                    .child(
                                        Button::new("clear-capture")
                                            .label("清空显示")
                                            .disabled(details.summaries.is_empty())
                                            .on_click(cx.listener(|player, _, _, cx| {
                                                player.diagnostics.summaries.clear();
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        Switch::new("capture-diagnostics")
                            .label("启用额外采集诊断")
                            .checked(self.settings.capture_diagnostics)
                            .disabled(self.locked())
                            .on_click(cx.listener(|player, enabled, window, cx| {
                                player.save_settings(
                                    Settings {
                                        capture_diagnostics: *enabled,
                                        ..player.settings.clone()
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        div().text_sm().text_color(muted).child(
                            "下次连接生效，显示最近 120 条摘要。逐包文件保存时序与处理耗时。",
                        ),
                    )
                    .when_some(details.capture_path.clone(), |el, path| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(format!("逐包：{path}")),
                        )
                    })
                    .when(details.summaries.is_empty(), |el| {
                        el.child(div().text_sm().text_color(muted).child(
                            if details.extra_capture {
                                "等待采集诊断输出。"
                            } else {
                                "未启用采集诊断。"
                            },
                        ))
                    })
                    .when(!details.summaries.is_empty(), |el| {
                        el.child(self.log_lines(Lines::Capture, cx))
                    }),
            )
    }

    fn queue(&self, kind: Lines) -> &VecDeque<String> {
        match kind {
            Lines::Logs => &self.diagnostics.logs,
            Lines::Capture => &self.diagnostics.summaries,
            Lines::Report => unreachable!(),
        }
    }

    fn copy_lines_button(
        &self,
        id: &'static str,
        label: &'static str,
        kind: Lines,
        cx: &Context<Self>,
    ) -> Button {
        let empty = match kind {
            Lines::Report => self.diagnostics.report.is_none(),
            _ => self.queue(kind).is_empty(),
        };
        Button::new(id)
            .label(label)
            .disabled(empty)
            .on_click(cx.listener(move |player, _, _, cx| {
                let text = match kind {
                    Lines::Report => player
                        .diagnostics
                        .report
                        .as_ref()
                        .map_or_else(String::new, |report| report.text.clone()),
                    _ => player
                        .queue(kind)
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }))
    }

    fn log_lines(&self, kind: Lines, cx: &Context<Self>) -> impl IntoElement {
        let (id, length, longest) = match kind {
            Lines::Report => {
                let mut length = 0;
                let mut longest = (0, 0);
                if let Some(report) = &self.diagnostics.report {
                    for (index, line) in report.text.lines().enumerate() {
                        length += 1;
                        if line.len() > longest.1 {
                            longest = (index, line.len());
                        }
                    }
                }
                ("report-lines", length, Some(longest.0))
            }
            _ => {
                let queue = self.queue(kind);
                let longest = queue
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, line)| line.len())
                    .map(|(index, _)| index);
                let id = if matches!(kind, Lines::Logs) {
                    "log-lines"
                } else {
                    "capture-lines"
                };
                (id, queue.len(), longest)
            }
        };
        uniform_list(
            id,
            length,
            cx.processor(
                move |player, range: std::ops::Range<usize>, _, _| match kind {
                    Lines::Report => {
                        player
                            .diagnostics
                            .report
                            .as_ref()
                            .map_or_else(Vec::new, |report| {
                                report
                                    .text
                                    .lines()
                                    .skip(range.start)
                                    .take(range.len())
                                    .map(log_row)
                                    .collect()
                            })
                    }
                    _ => range
                        .filter_map(|index| player.queue(kind).get(index))
                        .map(|line| log_row(line))
                        .collect(),
                },
            ),
        )
        .with_width_from_item(longest)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .h(px((length as f32 * 22.).clamp(44., 220.)))
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
    }
}

fn scroll_page(id: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_3()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
}

fn panel(cx: &Context<Player>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .p_4()
}

fn heading(title: &'static str) -> Div {
    div().font_semibold().child(title)
}

fn metric(label: impl Into<gpui::SharedString>, value: String) -> Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .text_sm()
        .child(div().w(px(176.)).flex_shrink_0().child(label.into()))
        .child(div().flex_1().min_w_0().child(value))
}

fn stat_row(values: [String; 7]) -> Div {
    div()
        .flex()
        .gap_2()
        .text_sm()
        .children(values.into_iter().enumerate().map(|(index, value)| {
            let cell = div().min_w_0().child(value);
            if index == 0 {
                cell.w(px(112.)).flex_shrink_0()
            } else {
                cell.flex_1()
            }
        }))
}

fn decimal(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "—".into(), |n| format!("{n:.decimals$}"))
}

fn count(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), |n| n.to_string())
}

fn audio_format(format: Option<&Format>) -> String {
    let Some(format) = format.and_then(|format| serde_json::to_value(format).ok()) else {
        return "设备未公布格式".into();
    };
    let bits = format["valid_bits"]
        .as_u64()
        .filter(|bits| *bits > 0)
        .or_else(|| format["bits"].as_u64());
    let encoding = match format["encoding"].as_str() {
        Some("pcm") => "PCM",
        Some("float32") => "float",
        Some(other) => other,
        None => "—",
    };
    format!(
        "{} Hz · {}-bit {encoding} · {} 声道",
        count(format["rate"].as_u64()),
        count(bits),
        count(format["channels"].as_u64())
    )
}

fn level(value: f32) -> String {
    if value > 0. {
        format!("{:.1} dBFS", 20. * value.log10())
    } else {
        "−∞ dBFS".into()
    }
}

fn log_row(text: &str) -> Div {
    div()
        .h(px(22.))
        .px_2()
        .text_xs()
        .font_family("Consolas")
        .whitespace_nowrap()
        .child(text.replace(['\r', '\n'], " ↵ "))
}
