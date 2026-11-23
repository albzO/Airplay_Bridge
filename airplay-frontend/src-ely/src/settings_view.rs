//! 设置页界面：常规、运行统计、技术详情与日志。
//! Settings views: general, runtime statistics, technical details and logs.
use crate::{
    lang::{self, t},
    panel::{Card, CardHeader, GAP, INSET, PANEL_RADIUS, Rows, TIGHT},
    player::Player,
    tf,
    view::{GHOST_PAD, audio_format, click, dbfs, edit, grouped, mono, pick, small},
};
use airplay_desktop::{Appearance, Settings};
use ely_core::{
    buttons::{Button, ButtonVariant, SegmentedControl},
    data_display::{Badge, Sparkline, Tone},
    forms::{Choice, NumberInput, Select, Switch},
    layout::{Accordion, AccordionItem},
    primitives::{Icon, IconName},
    theme::{ActiveTheme, ControlSize, TextSize},
};
use gpui::{
    App, ClipboardItem, FontWeight, IntoElement, ParentElement, Styled, WeakEntity, div,
    prelude::*, px,
};

/// 报告在界面中最多显示的行数；完整内容可用“复制报告”取得。
/// Report lines shown in the view; "copy report" yields the full text.
const REPORT_LINES: usize = 400;

fn decimal(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "—".into(), |n| format!("{n:.decimals$}"))
}

fn count(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), grouped)
}

fn setting_row(
    title: &'static str,
    detail: Option<String>,
    tag: Option<&'static str>,
    control: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .flex_1()
                // 长说明在此换行，控件始终留在卡片内。
                // Long notes wrap here so the control always stays inside the card.
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(theme.text_size(TextSize::Sm))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title)
                        .children(tag.map(|tag| Badge::new(tag).tone(Tone::Info))),
                )
                .children(detail.map(|detail| small(detail, cx))),
        )
        .child(div().flex_none().child(control))
}

fn note(text: &'static str) -> Option<String> {
    Some(text.to_owned())
}

impl Player {
    pub(crate) fn general(&self, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let locked = self.settings_locked();
        let channels = self
            .source()
            .and_then(|source| source.channels)
            .unwrap_or(0) as usize;
        let channel = |id: &'static str, side: usize| {
            Select::new(
                id,
                (0..channels).map(|ch| {
                    Choice::new(
                        ch.to_string(),
                        tf!("输入声道 {}", "Input channel {}", ch + 1),
                    )
                }),
            )
            .placeholder(t("未选择来源", "No source"))
            .selected(self.settings.mapping[side].to_string())
            .size(ControlSize::Sm)
            .disabled(channels == 0 || locked)
            .on_change(pick(this, move |player, value, window, cx| {
                if let Ok(channel) = value.parse() {
                    player.set_mapping(side, channel, window, cx);
                }
            }))
        };
        let appearance = match self.settings.appearance {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        };
        let behavior = Card::new()
            .header(CardHeader::new(t("应用行为", "App behavior")))
            .child(setting_row(
                t("开机自启", "Start at login"),
                note(t("登录 Windows 后启动 AirPlay Hub。启用后请保留软件所在位置。", "Start AirPlay Hub when you sign in to Windows. Keep the app in its current folder.")),
                None,
                Switch::new("autostart", self.autostart)
                    .disabled(self.operation.is_some() || self.engine.is_quitting())
                    .on_change(edit(this, |player, enabled: bool, window, cx| {
                        player.change_autostart(enabled, window, cx)
                    })),
                cx,
            ))
            .child(setting_row(
                t("串流中保持系统唤醒", "Keep awake while streaming"),
                note(t("串流期间阻止自动睡眠和熄屏，停止串流后恢复。", "Prevents automatic sleep and display-off during a stream; restored when it stops.")),
                None,
                Switch::new("awake", self.settings.keep_awake)
                    .disabled(locked)
                    .on_change(edit(this, |player, keep_awake: bool, window, cx| {
                        player.save_settings(
                            Settings {
                                keep_awake,
                                ..player.settings.clone()
                            },
                            window,
                            cx,
                        );
                    })),
                cx,
            ))
            .child(setting_row(
                t("右上角关闭按钮", "Close button"),
                note(t("立即生效。托盘右键“退出”始终关闭应用。", "Takes effect immediately. Quit in the tray menu always closes the app.")),
                None,
                SegmentedControl::new("close", self.settings.close_action.clone())
                    .segment("tray", t("收起到托盘", "Hide to tray"), None)
                    .segment("quit", t("退出应用", "Quit"), None)
                    .size(ControlSize::Sm)
                    .on_change(pick(this, |player, value, window, cx| {
                        if player.settings.close_action != value {
                            player.save_settings(
                                Settings {
                                    close_action: value.into(),
                                    ..player.settings.clone()
                                },
                                window,
                                cx,
                            );
                        }
                    })),
                cx,
            ))
            .child(setting_row(
                t("外观", "Appearance"),
                None,
                None,
                div().w(px(252.)).child(
                    SegmentedControl::new("appearance", appearance)
                        .segment("system", t("跟随系统", "System"), Some(IconName::Monitor))
                        .segment("light", t("浅色", "Light"), Some(IconName::Sun))
                        .segment("dark", t("深色", "Dark"), Some(IconName::Moon))
                        .size(ControlSize::Sm)
                        .on_change(pick(this, |player, value, window, cx| {
                            let appearance = match value {
                                "light" => Appearance::Light,
                                "dark" => Appearance::Dark,
                                _ => Appearance::System,
                            };
                            if player.settings.appearance != appearance {
                                player.save_settings(
                                    Settings {
                                        appearance,
                                        ..player.settings.clone()
                                    },
                                    window,
                                    cx,
                                );
                            }
                        })),
                ),
                cx,
            ))
            .child(setting_row(
                t("语言", "Language"),
                note(t(
                    "立即生效。托盘菜单的文字在下次启动后更新。",
                    "Takes effect immediately. The tray menu updates after a restart.",
                )),
                None,
                div().w(px(252.)).child(
                    SegmentedControl::new("language", self.language.key())
                        .segment("system", t("跟随系统", "System"), None)
                        .segment("zh", "中文", None)
                        .segment("en", "English", None)
                        .size(ControlSize::Sm)
                        .on_change(pick(this, |player, value, _, cx| {
                            if let Some(choice) = lang::Choice::from_key(value) {
                                player.set_language(choice, cx);
                            }
                        })),
                ),
                cx,
            ));
        let playback = Card::new().header(CardHeader::new(t("播放设置", "Playback"))).child(setting_row(
            t("播放提前量", "Playback lead"),
            note(t("请求的协议播放时间偏移（250–2000 ms），实际值受设备能力限制，不会同步外部视频或歌词。", "Requested protocol playback offset (250–2000 ms). The device may limit the actual value; it does not sync external video or lyrics.")),
            Some(t("下次连接生效", "Next connection")),
            div().w(px(140.)).child(
                NumberInput::new("latency", f64::from(self.settings.latency))
                    .range(250., 2000.)
                    .step(10.)
                    .suffix(small("ms", cx))
                    .size(ControlSize::Sm)
                    .on_commit(edit(this, |player, value: f64, window, cx| {
                        player.save_stream_settings(
                            Settings {
                                latency: value.round() as u32,
                                ..player.settings.clone()
                            },
                            window,
                            cx,
                        );
                    })),
            ),
            cx,
        ));
        let mapping = Card::new()
            .header(
                CardHeader::new(t("输入声道映射", "Input channel mapping"))
                    .description(t("选择音频来源中的左右输入。交换扬声器位置不会改变这里的设置。", "Pick the source's left and right inputs. Swapping speaker positions does not change this.")),
            )
            .child(setting_row(
                t("左输出取样", "Left output from"),
                None,
                None,
                div().w(px(140.)).child(channel("map-l", 0)),
                cx,
            ))
            .child(setting_row(
                t("右输出取样", "Right output from"),
                None,
                None,
                div().w(px(140.)).child(channel("map-r", 1)),
                cx,
            ));
        div()
            .flex()
            .flex_col()
            .gap(GAP)
            .child(behavior)
            .child(playback)
            .child(mapping)
    }

    pub(crate) fn runtime(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let telemetry = &self.diagnostics.telemetry;
        let report = self.diagnostics.report.as_ref();
        let tile = |label: &'static str, value: String, unit: &'static str, icon: IconName| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_2()
                .p(INSET)
                .rounded(PANEL_RADIUS)
                .border_1()
                .border_color(c.border)
                .bg(c.surface)
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(small(label, cx))
                        .child(Icon::new(icon).color(c.fg_subtle)),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_1()
                        .child(
                            div()
                                .text_size(theme.text_size(TextSize::Xxl))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(value),
                        )
                        .child(small(unit, cx).mb(px(5.))),
                )
        };
        let tiles = div()
            .flex()
            .gap(INSET)
            .child(tile(
                t("运行时长", "Elapsed"),
                decimal(telemetry.elapsed_seconds, 0),
                "s",
                IconName::Clock,
            ))
            .child(tile(
                t("流水线水位", "Pipeline level"),
                decimal(telemetry.water_ms, 1),
                "ms",
                IconName::Gauge,
            ))
            .child(tile(
                t("漂移校正", "Drift correction"),
                decimal(telemetry.controller.correction_ppm, 1),
                "ppm",
                IconName::Activity,
            ));
        let delay = telemetry
            .lead_ms
            .zip(telemetry.water_ms)
            .map(|(lead, water)| lead + water);
        let pending = || t("停止后汇总", "Summarized after stop").to_owned();
        let rows = Rows::new()
            .text(
                t("采集帧数", "Captured frames"),
                count(telemetry.capture_frames),
            )
            .text(
                t("输出帧数", "Output frames"),
                count(telemetry.output_frames),
            )
            .text(
                t("协议提前量", "Protocol lead"),
                format!("{} ms", decimal(telemetry.lead_ms, 0)),
            )
            .text(
                t("估计发送端延迟", "Estimated sender delay"),
                format!("{} ms", decimal(delay, 1)),
            )
            .text(
                t(
                    "采集不连续 / 时间戳错误",
                    "Discontinuities / timestamp errors",
                ),
                report.map_or_else(pending, |report| {
                    format!(
                        "{} / {}",
                        count(report.discontinuities),
                        count(report.timestamp_errors)
                    )
                }),
            )
            .text(
                t("转换汇总", "Conversion summary"),
                report
                    .and_then(|report| report.conversion.clone())
                    .unwrap_or_else(pending),
            );
        let pipeline = Card::new()
            .header(
                CardHeader::new(t("音频流水线", "Audio pipeline")).description(t("延迟估计不包含应用和扬声器实际发声耗时。", "The delay estimate excludes the playing app and the speaker's own output time.")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when(self.water.len() > 1, |el| {
                        el.child(
                            Sparkline::new(self.water.iter().copied())
                                .area()
                                .color(c.accent)
                                .w_full()
                                .h(px(40.)),
                        )
                    })
                    .child(rows),
            );
        let header = [
            t("设备", "Device"),
            t("发送包", "Sent"),
            t("本地丢包", "Dropped"),
            t("同步丢包", "Sync dropped"),
            t("重传请求", "RTX asked"),
            t("已重传", "Resent"),
            t("过期", "Expired"),
        ];
        let cell = |text: String, first: bool, strong: bool| {
            div()
                .when(first, |el| el.flex_1().min_w_0().truncate())
                .when(first && strong, |el| el.font_weight(FontWeight::MEDIUM))
                .when(!first, |el| el.w(px(72.)).flex_none().text_right())
                .text_size(theme.text_size(TextSize::Sm))
                .text_color(if strong { c.fg } else { c.fg_muted })
                .child(text)
        };
        let stats = self.diagnostics.devices.iter().map(|device| {
            div()
                .flex()
                .py_2()
                .border_t_1()
                .border_color(c.border)
                .child(cell(device.name.clone(), true, true))
                .children(
                    device
                        .totals
                        .iter()
                        .enumerate()
                        .map(|(index, total)| cell(count(*total), false, index == 0)),
                )
        });
        let transfer = Card::new()
            .header(CardHeader::new(t("设备传输", "Device transfer")).description(
                t("约每 5 秒更新，累计本次运行中最近 128 个设备的统计，退出软件后清空。", "Updates about every 5 seconds. Totals cover the last 128 devices in this run and clear when the app quits."),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().flex().pb_2().children(header.iter().enumerate().map(
                        |(index, title)| {
                            cell((*title).into(), index == 0, false)
                                .text_size(theme.text_size(TextSize::Xs))
                        },
                    )))
                    .children(stats)
                    .when(self.diagnostics.devices.is_empty(), |el| {
                        el.child(
                            div()
                                .py_2()
                                .border_t_1()
                                .border_color(c.border)
                                .child(small(t("等待接收端传输统计。", "Waiting for transfer statistics from the receiver."), cx)),
                        )
                    }),
            );
        div()
            .flex()
            .flex_col()
            .gap(GAP)
            .child(tiles)
            .child(pipeline)
            .child(transfer)
    }

    pub(crate) fn technical(&self, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let source = self.source();
        let details = &self.diagnostics;
        let capture = Card::new()
            .header(CardHeader::new(t("采集与处理", "Capture and processing")).description(
                t("默认格式与 WASAPI 实际工作格式可能不同；共享采集流常为 float32。", "The default format and the format WASAPI actually runs can differ; shared capture streams are often float32."),
            ))
            .child(
                Rows::new()
                    .text(
                        t("设备格式", "Device format"),
                        audio_format(source.and_then(|source| source.device_format.as_ref())),
                    )
                    .text(
                        t("WASAPI 采集格式", "WASAPI capture format"),
                        audio_format(source.and_then(|source| source.mix_format.as_ref())),
                    )
                    .text(
                        t("本次采集采样率", "Capture sample rate"),
                        details
                            .telemetry
                            .input_rate
                            .map_or_else(|| "—".into(), |rate| format!("{rate} Hz")),
                    )
                    .text(t("发送格式", "Send format"), "44100 Hz · 16-bit PCM → ALAC")
                    .text(
                        t("左右电平", "Left / right level"),
                        format!("{} / {}", dbfs(self.peaks[0]), dbfs(self.peaks[1])),
                    ),
            )
            .child(setting_row(
                "Buffer",
                note(t("发送端 PCM 预读深度（64–512 ms），与播放提前量不同。", "Sender PCM read-ahead (64–512 ms). Not the same as playback lead.")),
                Some(t("下次连接生效", "Next connection")),
                div().w(px(140.)).child(
                    NumberInput::new("buffer", f64::from(self.settings.buffer))
                        .range(64., 512.)
                        .step(16.)
                        .suffix(small("ms", cx))
                        .size(ControlSize::Sm)
                        .on_commit(edit(this, |player, value: f64, window, cx| {
                            player.save_stream_settings(
                                Settings {
                                    buffer: value.round() as u32,
                                    ..player.settings.clone()
                                },
                                window,
                                cx,
                            );
                        })),
                ),
                cx,
            ))
            .child(setting_row(
                t("密码要求记录", "Password requirement memory"),
                Some(self.auth_notice.clone().unwrap_or_else(|| {
                    t("只记住密码要求，不保存密码。取消设备密码后可重新检测。", "Remembers only whether a password is required, never the password. Re-detect after removing a device's password.").into()
                })),
                None,
                Button::new("reset-auth", t("重新检测所选设备", "Re-detect selected device"))
                    .size(ControlSize::Sm)
                    .disabled(self.devices_locked() || self.receiver().is_none())
                    .on_click(click(this, |player, window, cx| {
                        player.reset_auth(window, cx)
                    })),
                cx,
            ));
        let dash = |value: &Option<String>| value.clone().unwrap_or_else(|| "—".into());
        let protocol = Card::new()
            .header(CardHeader::new(t("会话协议", "Session protocol")))
            .child(
                Rows::new()
                    .text(t("时钟协议", "Clock protocol"), dash(&details.clock))
                    .text(t("认证方式", "Authentication"), dash(&details.auth)),
            );
        let capabilities = Card::new()
            .header(
                CardHeader::new(t("设备能力", "Device capabilities")).description(t(
                    "mDNS 发现的 TXT 属性",
                    "TXT properties from mDNS discovery",
                )),
            )
            .child(if self.devices.is_empty() {
                small(t("尚未发现接收端。", "No receivers found yet."), cx).into_any_element()
            } else {
                let mut accordion = Accordion::new("capabilities").multiple();
                for device in &self.devices {
                    let addresses = device
                        .addresses
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut rows = Rows::new();
                    for (field, value) in &device.properties {
                        rows = rows.text(field.clone(), value.clone());
                    }
                    accordion = accordion.item(
                        AccordionItem::new(format!(
                            "{} · {addresses}:{}",
                            device.name, device.port
                        ))
                        .child(div().pb_2().child(rows)),
                    );
                }
                accordion.into_any_element()
            });
        div()
            .flex()
            .flex_col()
            .gap(GAP)
            .child(capture)
            .child(protocol)
            .child(capabilities)
    }

    pub(crate) fn logs(&self, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let locked = self.locked();
        let details = &self.diagnostics;
        let copy =
            |id: &'static str, label: &'static str, empty: bool, text: fn(&Player) -> String| {
                Button::new(id, label)
                    .icon(IconName::Copy)
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(empty)
                    .on_click(click(this, move |player, _, cx| {
                        let text = text(player);
                        if !text.is_empty() {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }))
            };
        fn joined(lines: &std::collections::VecDeque<String>) -> String {
            lines
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        }
        let session = Card::new()
            .header(
                CardHeader::new(t("会话日志", "Session log")).action(
                    div()
                        .flex()
                        .gap_2()
                        .child(copy("copy-log", t("复制日志", "Copy log"), details.logs.is_empty(), |player| {
                            joined(&player.diagnostics.logs)
                        }))
                        .child(
                            Button::new("open-logs", t("打开日志目录", "Open log folder"))
                                .icon(IconName::FolderOpen)
                                .size(ControlSize::Sm)
                                .on_click(click(this, |player, _, cx| {
                                    if let Err(error) = player.engine.open_logs() {
                                        player.message = Some(error);
                                        cx.notify();
                                    }
                                })),
                        ),
                ),
            )
            .child(setting_row(
                t("保存详细日志", "Save detailed logs"),
                note(t("开启后保存详细协议日志和每秒采集、重采样、队列、发送进度；关闭时只保存关键故障。", "When on, saves detailed protocol logs and per-second capture, resampling, queue and send progress. When off, only critical faults are saved.")),
                Some(t("下次连接生效", "Next connection")),
                Switch::new("detailed", self.settings.detailed_logs)
                    .disabled(locked)
                    .on_change(edit(this, |player, detailed_logs: bool, window, cx| {
                        player.save_settings(
                            Settings {
                                detailed_logs,
                                ..player.settings.clone()
                            },
                            window,
                            cx,
                        );
                    })),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(TIGHT)
                    .child(small(details.log_path.clone(), cx))
                    .children(
                        details
                            .pipeline_path
                            .clone()
                            .map(|path| small(tf!("流水线：{path}", "Pipeline: {path}"), cx)),
                    )
                    .child(small(
                        if details.detailed {
                            t("显示最近 300 条详细输出。文件有大小与保留限制，旧日志会清理。", "Showing the last 300 detailed lines. Files are size- and retention-limited; old logs are removed.")
                        } else {
                            t("显示最近 300 条关键故障和告警。", "Showing the last 300 critical faults and warnings.")
                        },
                        cx,
                    ))
                    .child(if details.logs.is_empty() {
                        mono(
                            "log-lines",
                            [if details.detailed {
                                t("等待详细日志输出。", "Waiting for detailed log output.")
                            } else {
                                t("暂无关键故障。", "No critical faults.")
                            }
                            .to_owned()],
                            cx,
                        )
                        .into_any_element()
                    } else {
                        mono(
                            "log-lines",
                            details.logs.iter().map(|line| line.replace(['\r', '\n'], " ↵ ")),
                            cx,
                        )
                        .into_any_element()
                    }),
            )
            .children(details.report.as_ref().map(|report| {
                div()
                    .flex()
                    .flex_col()
                    .gap(TIGHT)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(cx.theme().text_size(TextSize::Sm))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(t("完整会话报告", "Full session report")),
                            )
                            .child(div().mr(GHOST_PAD).child(copy(
                                "copy-report",
                                t("复制报告", "Copy report"),
                                false,
                                |player| {
                                    player
                                        .diagnostics
                                        .report
                                        .as_ref()
                                        .map_or_else(String::new, |report| report.text.clone())
                                },
                            ))),
                    )
                    .child(mono(
                        "report-lines",
                        report.text.lines().take(REPORT_LINES).map(str::to_owned),
                        cx,
                    ))
            }));
        let diagnostics = Card::new()
            .header(
                CardHeader::new(t("采集逐包诊断", "Per-packet capture diagnostics")).action(
                    div()
                        .flex()
                        .gap_2()
                        .mr(GHOST_PAD)
                        .child(copy(
                            "copy-capture",
                            t("复制摘要", "Copy summary"),
                            details.summaries.is_empty(),
                            |player| joined(&player.diagnostics.summaries),
                        ))
                        .child(
                            Button::new("clear-capture", t("清空显示", "Clear"))
                                .icon(IconName::Eraser)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .disabled(details.summaries.is_empty())
                                .on_click(click(this, |player, _, cx| {
                                    player.diagnostics.summaries.clear();
                                    cx.notify();
                                })),
                        ),
                ),
            )
            .child(setting_row(
                t("启用额外采集诊断", "Enable extra capture diagnostics"),
                note(t("独立于详细日志。保存逐包时序、帧数、内容指纹与处理耗时，不保存原始音频；滚动保留当前和前 3 段，每段约 8 MiB。", "Independent of detailed logs. Saves per-packet timing, frame counts, content fingerprints and processing time, never raw audio. Keeps the current and previous 3 segments of about 8 MiB each.")),
                Some(t("下次连接生效", "Next connection")),
                Switch::new("capture-diag", self.settings.capture_diagnostics)
                    .disabled(locked)
                    .on_change(edit(this, |player, capture_diagnostics: bool, window, cx| {
                        player.save_settings(
                            Settings {
                                capture_diagnostics,
                                ..player.settings.clone()
                            },
                            window,
                            cx,
                        );
                    })),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(TIGHT)
                    .children(
                        details
                            .capture_path
                            .clone()
                            .map(|path| small(tf!("逐包：{path}", "Per-packet: {path}"), cx)),
                    )
                    .child(small(t("窗口每秒更新，保留最近 120 条摘要；逐包数据保存到文件。", "Updates every second and keeps the last 120 summaries; per-packet data goes to a file."), cx))
                    .child(if details.summaries.is_empty() {
                        mono(
                            "capture-lines",
                            [if details.extra_capture {
                                t("等待采集诊断输出。", "Waiting for capture diagnostics.")
                            } else {
                                t("未启用采集诊断。", "Capture diagnostics are off.")
                            }
                            .to_owned()],
                            cx,
                        )
                        .into_any_element()
                    } else {
                        mono("capture-lines", details.summaries.iter().cloned(), cx)
                            .into_any_element()
                    }),
            );
        div()
            .flex()
            .flex_col()
            .gap(GAP)
            .child(session)
            .child(diagnostics)
    }
}
