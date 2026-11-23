//! 播放页界面：标题栏、音频来源、AirPlay 设备、密码输入和底部固定栏。
//! Playback view: title bar, audio source, AirPlay devices, password prompt and the dock.
use crate::{
    lang::t,
    panel::{CONTROL_RADIUS, Card, CardHeader, GAP, GUTTER, INSET, PAGE_MAX, PANEL_RADIUS},
    player::{Op, Page, Player, Receiver},
    session::Phase,
    tf,
};
use airplay_desktop::{Appearance, Settings};
use ely_core::{
    buttons::{Button, ButtonVariant, IconButton},
    data_display::{Badge, Tone},
    feedback::Banner,
    forms::{Choice, PasswordInput, Slider, Switch},
    layout::Collapsible,
    motion::{self, BASE, Skeleton, SkeletonText, Spinner, ease_out_cubic},
    navigation::Tabs,
    primitives::{FocusScope, Icon, IconName, Severity, TooltipTrigger},
    shell::{WindowControls, drag_region},
    theme::{ActiveTheme, ControlSize, Platform, Radius, TextSize},
};
use gpui::{
    Animation, AnimationExt, App, ClickEvent, Context, Div, FontFeatures, FontWeight, Hsla,
    IntoElement, ParentElement, Render, SharedString, Stateful, Styled, WeakEntity, Window, canvas,
    div, prelude::*, px,
};
use homepod_test::capture::Format;
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

/// 视觉对齐：幽灵按钮没有可见边框，按其内边距向外偏移，使图标或文字贴齐内容边线。
/// Optical alignment: ghost buttons have no visible edge, so they shift out by their padding
/// and the glyph or label itself lands on the content edge.
pub(crate) const GHOST_PAD: gpui::Pixels = px(-8.);

/// 设置页标题行上方留白：让页眉底边（标签下划线）与首页标题区底边齐平。
/// Space above the settings title row, so the header's bottom edge (the tab underline) is level
/// with the bottom of the home title area.
const SETTINGS_TOP: f32 = 44.;
/// 标题行与标签之间的留白；与上面一起使标签下划线落在首页标题区底边（首页内容起点）。
/// Space between the title row and the tabs; with the above it puts the tab underline on the
/// bottom of the home title area, where the home content starts.
const SETTINGS_GAP: f32 = 6.;

/// 出现时淡入并上浮 4px；系统开启“减少动画”时直接显示。新出现的 `id` 才会播放。
/// Fades in and rises 4px on arrival; shown at once under reduced motion. Plays only for an `id`
/// not seen before.
pub(crate) fn arrive<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<gpui::ElementId>,
    cx: &App,
) -> impl IntoElement {
    let animation = Animation::new(motion::duration(BASE, cx)).with_easing(ease_out_cubic);
    element.with_animation(id, animation, |element, t| {
        element.opacity(t).mt(px(4. * (1. - t)))
    })
}

/// 布局后把元素高度写入 `cell`；铺满父元素、不参与布局。
/// Writes the element's height into `cell` after layout; fills its parent outside the flow.
pub(crate) fn measure(cell: Rc<Cell<f32>>) -> impl IntoElement {
    canvas(
        move |bounds, _, _| cell.set(f32::from(bounds.size.height)),
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// 搜索结果出现时，设备卡片自上而下依次落下。
/// When search results arrive, the device cards drop in one after another from above.
fn drop_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<gpui::ElementId>,
    index: usize,
    cx: &App,
) -> impl IntoElement {
    let each = motion::duration(BASE, cx).as_secs_f32();
    let delay = if each > 0.01 { 0.05 * index as f32 } else { 0. };
    let total = each + delay;
    let animation = Animation::new(Duration::from_secs_f32(total.max(0.001)));
    element.with_animation(id, animation, move |element, t| {
        let local = ((t * total - delay) / each.max(0.001)).clamp(0., 1.);
        let e = ease_out_cubic(local);
        element.opacity(e).top(px(-14. * (1. - e)))
    })
}

/// 只淡入，不位移：设置页内换标签时用。
/// Fades in without moving; used when the settings tab changes.
fn fade_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<gpui::ElementId>,
    cx: &App,
) -> impl IntoElement {
    let animation = Animation::new(motion::duration(BASE, cx)).with_easing(ease_out_cubic);
    element.with_animation(id, animation, |element, t| element.opacity(t))
}

/// 进入设置：标题栏、标签和内容作为一整块平面从上方平移进来，没有回弹。
/// Entering settings: the title bar, tabs and content move in from above as one plane, with no
/// overshoot.
fn plane_in<E: IntoElement + Styled + 'static>(element: E, cx: &App) -> impl IntoElement {
    let animation = Animation::new(motion::duration(motion::SLOW, cx)).with_easing(ease_out_cubic);
    element.with_animation("settings-plane", animation, |element, t| {
        element.opacity(t).top(px(-32. * (1. - t)))
    })
}

/// 把播放器方法包装成 Ely 按钮所需的点击回调。
/// Wraps a player method as the click handler Ely buttons take.
pub(crate) fn click(
    this: &WeakEntity<Player>,
    action: impl Fn(&mut Player, &mut Window, &mut Context<Player>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let this = this.clone();
    move |_, window, cx| {
        let _ = this.update(cx, |player, cx| action(player, window, cx));
    }
}

/// 把播放器方法包装成开关、滑块、数字输入的值回调。
/// Wraps a player method as the value callback switches, sliders and number inputs take.
pub(crate) fn edit<T: 'static>(
    this: &WeakEntity<Player>,
    action: impl Fn(&mut Player, T, &mut Window, &mut Context<Player>) + 'static,
) -> impl Fn(T, &mut Window, &mut App) + 'static {
    let this = this.clone();
    move |value, window, cx| {
        let _ = this.update(cx, |player, cx| action(player, value, window, cx));
    }
}

/// 把播放器方法包装成下拉、分段控件的选中值回调。
/// Wraps a player method as the picked-value callback selects and segmented controls take.
pub(crate) fn pick(
    this: &WeakEntity<Player>,
    action: impl Fn(&mut Player, &str, &mut Window, &mut Context<Player>) + 'static,
) -> impl Fn(&SharedString, &mut Window, &mut App) + 'static {
    let this = this.clone();
    move |value, window, cx| {
        let _ = this.update(cx, |player, cx| action(player, value.as_ref(), window, cx));
    }
}

pub(crate) fn small(text: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .text_size(theme.text_size(TextSize::Xs))
        .text_color(theme.colors.fg_muted)
        .child(text.into())
}

/// 内容列：最宽 `PAGE_MAX`，窗口更宽时居中，两侧留 `GUTTER`。
/// The content column: at most `PAGE_MAX` wide, centered in wider windows, with `GUTTER` either side.
pub(crate) fn page() -> Div {
    div().w_full().max_w(PAGE_MAX).mx_auto().px(GUTTER)
}

/// 千位分隔：36336000 → 36,336,000。
/// Thousands separators: 36336000 becomes 36,336,000.
pub(crate) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// 线性峰值 → dBFS 文本。
/// Linear peak as dBFS text.
pub(crate) fn dbfs(peak: f32) -> String {
    if peak > 0. {
        format!("{:.1} dBFS", 20. * peak.log10())
    } else {
        "−∞ dBFS".into()
    }
}

/// 电平条宽度：−60 dBFS 到 0 dBFS 映射到 0–1。
/// Meter fill: −60 dBFS to 0 dBFS mapped onto 0–1.
fn fill(peak: f32) -> f32 {
    if peak > 0. {
        ((20. * peak.log10() + 60.) / 60.).clamp(0., 1.)
    } else {
        0.
    }
}

pub(crate) fn audio_format(format: Option<&Format>) -> String {
    let Some(format) = format.and_then(|format| serde_json::to_value(format).ok()) else {
        return t("设备未公布格式", "Format not reported").into();
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
    let number = |value: Option<u64>| value.map_or_else(|| "—".into(), |n| n.to_string());
    let channels = t("声道", "ch");
    format!(
        "{} Hz · {}-bit {encoding} · {} {channels}",
        number(format["rate"].as_u64()),
        number(bits),
        number(format["channels"].as_u64())
    )
}

/// 会话阶段在当前语言下的文字。
/// The session phase in the current language.
fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => t("尚未连接", "Not connected"),
        Phase::Connecting => t("正在连接", "Connecting"),
        Phase::Preparing => t("等待采集稳定", "Waiting for capture"),
        Phase::Password => t("需要 AirPlay 密码", "Password required"),
        Phase::Verifying => t("正在验证密码", "Verifying password"),
        Phase::Ready => t("正在启动音频", "Starting audio"),
        Phase::Playing => t("串流中", "Streaming"),
        Phase::Stopping => t("正在停止", "Stopping"),
        Phase::Stopped => t("已停止", "Stopped"),
        Phase::Failed => t("串流失败", "Streaming failed"),
    }
}

fn phase_tone(phase: Phase) -> Tone {
    match phase {
        Phase::Idle | Phase::Stopped => Tone::Neutral,
        Phase::Failed => Tone::Danger,
        Phase::Playing => Tone::Success,
        _ => Tone::Warning,
    }
}

impl Player {
    /// 顶栏即标题栏：整块可拖动窗口；窗口按钮贴在窗口右上角，标题随内容列居中。
    /// The header is the title bar: all of it drags the window; the window buttons sit in the
    /// window's top-right corner while the title follows the centered content column.
    fn header(&self, bar: Stateful<Div>, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let dark = theme.is_dark();
        let settings = self.page != Page::Playback;
        // 补丁号为 0 时只显示主次版本，如 2.0.0 显示为 2.0。
        // With a zero patch number show only major.minor, so 2.0.0 reads 2.0.
        let version = match env!("CARGO_PKG_VERSION_PATCH") {
            "0" => concat!(
                env!("CARGO_PKG_VERSION_MAJOR"),
                ".",
                env!("CARGO_PKG_VERSION_MINOR")
            ),
            _ => env!("CARGO_PKG_VERSION"),
        };
        let title = div().flex().items_center().gap_3().child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(theme.text_size(TextSize::Xl))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("AirPlay Hub"),
                )
                .child(small("Windows → AirPlay", cx)),
        );
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
            .h(theme.titlebar_height())
            .pr_2()
            .occlude()
            .child(
                IconButton::new("theme", if dark { IconName::Sun } else { IconName::Moon })
                    .variant(ButtonVariant::Ghost)
                    .size(ControlSize::Sm)
                    .tooltip(if dark {
                        t("浅色外观", "Light appearance")
                    } else {
                        t("深色外观", "Dark appearance")
                    })
                    .disabled(self.settings_locked())
                    .on_click(click(this, move |player, window, cx| {
                        let appearance = if dark {
                            Appearance::Light
                        } else {
                            Appearance::Dark
                        };
                        player.save_settings(
                            Settings {
                                appearance,
                                ..player.settings.clone()
                            },
                            window,
                            cx,
                        );
                    })),
            )
            .child(
                IconButton::new("settings", IconName::Settings)
                    .variant(if settings {
                        ButtonVariant::Subtle
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(ControlSize::Sm)
                    .tooltip(if settings {
                        t("返回播放", "Back to playback")
                    } else {
                        t("设置", "Settings")
                    })
                    .on_click(click(this, |player, _, cx| {
                        player.page = if player.page == Page::Playback {
                            player.last_tab
                        } else {
                            Page::Playback
                        };
                        cx.notify();
                    })),
            );
        let bar = bar.relative().flex_none().w_full();
        let bar = if settings {
            // 设置页：标题压成一行，与窗口按钮同高，和主页面的大标题区分开。
            // Settings: a one-line title level with the window buttons, unlike the main page's
            // large heading.
            let c = &theme.colors;
            bar.bg(c.surface).child(
                page()
                    .pt(px(SETTINGS_TOP))
                    .pb(px(SETTINGS_GAP))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().ml(GHOST_PAD).occlude().child(
                            IconButton::new("back", IconName::ArrowLeft)
                                .variant(ButtonVariant::Ghost)
                                .tooltip(t("返回播放", "Back to playback"))
                                .on_click(click(this, |player, _, cx| {
                                    player.page = Page::Playback;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Lg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t("设置", "Settings")),
                    )
                    .child(small(format!("v{version}"), cx).mt(px(3.))),
            )
        } else {
            bar.child(page().pt(px(44.)).pb_4().flex().child(title))
        };
        bar.child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .flex()
                .items_start()
                .child(tools)
                .child(WindowControls::new("window-controls").platform(Platform::Windows)),
        )
    }

    fn tabs(&self, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let c = &cx.theme().colors;
        let tabs = Tabs::new(
            "tabs",
            Page::TABS.map(|tab| Choice::new(tab.key(), tab.label())),
            self.page.key(),
        )
        .on_change(pick(this, |player, value, _, cx| {
            if let Some(tab) = Page::from_key(value) {
                player.page = tab;
                player.last_tab = tab;
                cx.notify();
            }
        }));
        div()
            .flex_none()
            .bg(c.surface)
            .border_b_1()
            .border_color(c.border)
            .child(page().child(tabs))
    }

    /// 分段电平：绿 / 黄 / 红三段，按 dBFS 映射。
    /// Segmented meter in green, amber and red over the dBFS scale.
    fn meter(&self, peak: f32, segments: usize, height: f32, cx: &App) -> impl IntoElement {
        let c = &cx.theme().colors;
        let lit = (fill(peak) * segments as f32).round() as usize;
        let (success, warning, danger, off) = (c.success, c.warning, c.danger, c.border);
        div()
            .flex_1()
            .flex()
            .gap(px(2.))
            .children((0..segments).map(move |i| {
                let share = i as f32 / segments as f32;
                let on: Hsla = if share >= 0.9 {
                    danger
                } else if share >= 0.75 {
                    warning
                } else {
                    success
                };
                div()
                    .flex_1()
                    .h(px(height))
                    .rounded(px(1.))
                    .bg(if i < lit { on } else { off })
            }))
    }

    /// 左右电平块：L/R 两条相隔 2px 组成一块，右侧给出总峰值。
    /// Stereo level block: L and R bars 2px apart read as one, the total peak on the right.
    fn level_block(&self, segments: usize, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let total = self.peaks[0].max(self.peaks[1]);
        let channel = |label: &'static str, side: usize| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(10.))
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c.fg_subtle)
                        .child(label),
                )
                .child(self.meter(self.peaks[side], segments, 8., cx))
        };
        div()
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(channel("L", 0))
                    .child(channel("R", 1)),
            )
            .child(
                div()
                    .w(px(88.))
                    .flex_none()
                    .text_right()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if total > 0. { c.fg } else { c.fg_subtle })
                    .child(dbfs(total)),
            )
    }

    fn source_card(&self, this: &WeakEntity<Self>, window: &Window, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let locked = self.locked();
        let select = self.source_picker(this, window, cx);
        let source = self.source();
        let switching = matches!(
            self.operation,
            Some(Op::TogglingCapture | Op::SavingSource | Op::PausingCapture | Op::ResumingCapture)
        );
        let capture_label = match (source, switching, self.capture_wanted()) {
            (None, ..) => t("待选择来源", "No source"),
            (_, true, _) => t("切换中…", "Switching…"),
            (_, _, true) => t("采集中", "Capturing"),
            _ => t("采集已关闭", "Capture off"),
        };
        // 电平块与下拉同为 6px 圆角、同宽，读作一个控件组。
        // The level well shares the select's 6px radius and width, so they read as one group.
        let well = div()
            .flex()
            .flex_col()
            .gap_2()
            .px(INSET)
            .py_3()
            .rounded(CONTROL_RADIUS)
            .bg(c.sunken)
            .child(self.level_block(48, cx))
            .children(self.source_message.clone().map(|message| {
                div()
                    .text_size(theme.text_size(TextSize::Xs))
                    .text_color(c.warning)
                    .child(message)
            }));
        Card::new()
            .header(
                CardHeader::new(t("音频来源", "Audio source")).action(
                    Switch::new("capture", source.is_some() && self.capture_wanted())
                        .label(capture_label)
                        .disabled(locked || source.is_none())
                        .on_change(edit(this, |player, _: bool, window, cx| {
                            player.toggle_capture(window, cx)
                        })),
                ),
            )
            .child(div().flex().flex_col().gap_2().child(select).child(well))
    }

    /// 接收端图标：单设备一个圆，立体声对两个错开的圆。
    /// Receiver glyph: one disc for a device, two offset discs for a stereo pair.
    fn glyph(pair: bool, color: Hsla, ring: Hsla) -> impl IntoElement {
        let disc = move |size: f32| div().size(px(size)).rounded_full().bg(color);
        div()
            .relative()
            .size(px(36.))
            .flex_none()
            .when(!pair, |el| {
                el.flex().items_center().justify_center().child(disc(22.))
            })
            .when(pair, |el| {
                el.child(div().absolute().top(px(3.)).left(px(2.)).child(disc(20.)))
                    .child(
                        div()
                            .absolute()
                            .top(px(12.))
                            .left(px(13.))
                            .rounded_full()
                            .border_2()
                            .border_color(ring)
                            .child(disc(20.)),
                    )
            })
    }

    fn device_row(
        &self,
        index: usize,
        receiver: &Receiver,
        this: &WeakEntity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let pair = receiver.members.len() == 2;
        let selected = self.selected.as_ref() == Some(&receiver.key);
        let expanded = self.expanded.as_ref() == Some(&receiver.key);
        // 发现设备时所有卡片都不可点；串流中只有已选设备可展开。
        // During discovery no card can be clicked; while streaming only the chosen one expands.
        let disabled = self.discovering || (self.locked() && !selected);
        let phase = self.session.phase;
        let active = selected && !matches!(phase, Phase::Idle | Phase::Stopped);
        let border = match (selected, phase) {
            (false, _) => c.border,
            (_, Phase::Playing) => c.success,
            (_, Phase::Failed) => c.danger,
            (_, Phase::Idle | Phase::Stopped) => c.border_strong,
            _ => c.warning,
        };
        let subtitle = if pair {
            t("立体声对", "Stereo pair").to_owned()
        } else {
            self.devices[receiver.members[0]]
                .addresses
                .first()
                .map_or_else(|| "—".to_owned(), ToString::to_string)
        };
        let radio = div()
            .size(px(16.))
            .flex_none()
            .rounded_full()
            .border_1()
            .border_color(if selected { c.fg } else { c.border_strong })
            .flex()
            .items_center()
            .justify_center()
            .when(selected, |el| {
                el.child(div().size(px(8.)).rounded_full().bg(c.fg))
            });
        let key = receiver.key.clone();
        let head = div()
            .id(("device", index))
            .flex()
            .items_center()
            .gap_3()
            .px(INSET)
            .py_3()
            .when(!disabled, |el| el.cursor_pointer().hover(|s| s.bg(c.hover)))
            .child(radio)
            .child(Self::glyph(pair, c.fg_subtle, c.surface))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Sm))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(receiver.label.clone()),
                    )
                    .child(small(subtitle, cx)),
            )
            .when(active, |el| {
                el.child(Badge::new(phase_label(phase)).tone(phase_tone(phase)).dot())
            })
            .child(Badge::new(if pair { "Stereo" } else { "AirPlay 2" }))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                })
                .color(c.fg_subtle),
            )
            .on_click(click(this, move |player, window, cx| {
                player.choose_receiver(&key, window, cx)
            }));
        let details = {
            let body = if selected && self.connected() {
                self.channels(receiver, this, cx).into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(receiver.members.iter().map(|&member| {
                        let device = &self.devices[member];
                        let address = device
                            .addresses
                            .first()
                            .map_or_else(|| "—".to_owned(), ToString::to_string);
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::Wifi).color(c.fg_subtle))
                            .child(small(
                                format!("{} · {address}:{}", device.name, device.port),
                                cx,
                            ))
                    }))
                    .into_any_element()
            };
            Collapsible::new(("device-details", index), expanded).child(
                div()
                    .px(INSET)
                    .py_3()
                    .border_t_1()
                    .border_color(c.border)
                    .child(body),
            )
        };
        let card = div()
            .relative()
            .rounded(CONTROL_RADIUS)
            .border_1()
            .border_color(border)
            .bg(c.surface)
            .overflow_hidden()
            .child(head)
            .child(details);
        // 变灰放在动画外层，出现动画的透明度不会覆盖它。
        // Dimming sits outside the animation so the entrance's opacity never overrides it.
        div().when(disabled, |el| el.opacity(0.5)).child(drop_in(
            card,
            ("device-in", self.devices_generation * 64 + index),
            index,
            cx,
        ))
    }

    /// 连接后展开：左右扬声器（或声道）。标签与名称并为一行，下方是电平；三列网格，中列为交换按钮。
    /// Expanded when connected: label and name share one line above each meter, on a three-column
    /// grid with the swap button in the middle column.
    fn channels(&self, receiver: &Receiver, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let pair = receiver.names.len() == 2;
        let speaker = |side: usize| {
            let member = if pair && self.settings.speakers_swapped {
                1 - side
            } else if pair {
                side
            } else {
                0
            };
            let label = match (side, pair) {
                (0, true) => t("左扬声器", "Left speaker"),
                (_, true) => t("右扬声器", "Right speaker"),
                (0, false) => t("左声道", "Left channel"),
                _ => t("右声道", "Right channel"),
            };
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(theme.text_size(TextSize::Xs))
                                .text_color(c.fg_muted)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(theme.text_size(TextSize::Sm))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(receiver.names[member].clone()),
                        )
                        .child(small(dbfs(self.peaks[side]), cx)),
                )
                .child(self.meter(self.peaks[side], 24, 6., cx))
        };
        div()
            .flex()
            .items_center()
            .gap_4()
            .child(speaker(0))
            .child(div().w(px(32.)).flex().justify_center().when(pair, |el| {
                el.child(
                    IconButton::new("swap", IconName::ChevronsLeftRight)
                        .variant(ButtonVariant::Outline)
                        .size(ControlSize::Sm)
                        .tooltip(t("交换扬声器位置", "Swap speaker positions"))
                        .disabled(self.settings_locked())
                        .on_click(click(this, |player, window, cx| {
                            player.swap_speakers(window, cx)
                        })),
                )
            }))
            .child(speaker(1))
    }

    fn devices_card(&self, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let discovering = self.discovering;
        let refresh = Button::new(
            "refresh",
            if discovering {
                t("发现中…", "Searching…")
            } else {
                t("刷新", "Refresh")
            },
        )
        .icon(IconName::RefreshCw)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .disabled(self.devices_locked() || !self.backend_available)
        .on_click(click(this, |player, window, cx| {
            player.message = None;
            player.refresh(window, cx);
        }));
        Card::new()
            .header(
                CardHeader::new(t("AirPlay 设备", "AirPlay devices"))
                    .description(tf!(
                        "局域网内发现 {} 台 · 点击设备查看详情",
                        "{} found on the network · click a device for details",
                        self.devices.len()
                    ))
                    .action(div().mr(GHOST_PAD).child(refresh)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(discovering, |el| el.child(self.searching(cx)))
                    .when(discovering && self.receivers.is_empty(), |el| {
                        el.children((0..2).map(|index| self.placeholder_row(index, cx)))
                    })
                    .when(!discovering && self.receivers.is_empty(), |el| {
                        el.child(small(
                            t("未发现设备。点击刷新，确保 HomePod 与电脑在同一局域网。", "No devices found. Refresh, and make sure the HomePod and this PC are on the same network."),
                            cx,
                        ))
                    })
                    .children(
                        self.receivers
                            .iter()
                            .enumerate()
                            .map(|(index, receiver)| self.device_row(index, receiver, this, cx)),
                    ),
            )
    }

    /// 搜索提示：转圈加文字，搜索期间一直显示。
    /// The search notice: a spinner and a line, shown for the whole search.
    fn searching(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .items_center()
            .gap_2()
            .px(INSET)
            .py_1()
            .child(Spinner::new("searching").color(theme.colors.accent))
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .text_color(theme.colors.fg_muted)
                    .child(t(
                        "正在搜索局域网中的 AirPlay 设备…",
                        "Searching the network for AirPlay devices…",
                    )),
            )
    }

    /// 还没有设备时的占位卡片，闪烁表示正在加载。
    /// A shimmering placeholder card while no device is known yet.
    fn placeholder_row(&self, index: usize, cx: &App) -> impl IntoElement {
        let c = &cx.theme().colors;
        div()
            .flex()
            .items_center()
            .gap_3()
            .px(INSET)
            .py_3()
            .rounded(CONTROL_RADIUS)
            .border_1()
            .border_color(c.border)
            .bg(c.surface)
            .child(
                Skeleton::new(("placeholder-dot", index))
                    .circle()
                    .size(px(16.)),
            )
            .child(
                Skeleton::new(("placeholder-icon", index))
                    .circle()
                    .size(px(28.)),
            )
            .child(
                div()
                    .w(px(200.))
                    .child(SkeletonText::new(("placeholder-text", index), 2)),
            )
    }

    fn playback(&self, this: &WeakEntity<Self>, window: &Window, cx: &App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(GAP)
            .child(self.source_card(this, window, cx))
            .child(self.devices_card(this, cx))
    }

    /// 密码提示：固定在底部栏上方，回车或按钮提交；密码错误后提示重试。
    /// Password prompt pinned above the dock; Enter or the button submits, and a rejected
    /// password asks for another try.
    fn password_prompt(&self, name: String, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let rejected = self.session.phase == Phase::Failed;
        let empty = self.password.read(cx).is_empty();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .mb_3()
            .p(INSET)
            .rounded(PANEL_RADIUS)
            .border_1()
            .border_color(if rejected { c.danger } else { c.border_strong })
            .bg(c.surface)
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tf!(
                        "输入 {name} 的 AirPlay 密码",
                        "Enter the AirPlay password for {name}"
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(PasswordInput::new(&self.password)))
                    .child(
                        Button::new("password-submit", t("确认连接", "Connect"))
                            .primary()
                            .disabled(empty)
                            .on_click(click(this, |player, window, cx| {
                                player.submit_password(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .when(rejected, |el| {
                        el.child(
                            div()
                                .text_size(theme.text_size(TextSize::Xs))
                                .text_color(c.danger)
                                .child(t(
                                    "密码不正确，请重新输入。",
                                    "Incorrect password. Try again.",
                                )),
                        )
                    })
                    .child(small(
                        t(
                            "仅在本次会话使用，不保存密码。",
                            "Used for this session only; the password is not saved.",
                        ),
                        cx,
                    )),
            )
    }

    /// 底部固定栏：左右两区等宽，音量居中；左缘与卡片内文字对齐，右缘与卡片内控件对齐。
    /// The dock: equal left and right zones with the volume centered; its left edge lines up with
    /// text inside the cards and its right edge with the controls inside them.
    fn dock(&self, this: &WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let phase = self.session.phase;
        let playing = phase == Phase::Playing && !self.engine.is_quitting();
        let target = self.receiver().map_or_else(
            || t("未选择设备", "No device selected").to_owned(),
            |receiver| receiver.label.clone(),
        );
        let status = match (self.operation, phase) {
            (Some(operation), _) => operation.label().to_owned(),
            (None, Phase::Idle) if self.discovering => {
                t("正在发现接收端", "Searching for receivers").to_owned()
            }
            (None, Phase::Playing) => match self.diagnostics.telemetry.elapsed_seconds {
                Some(elapsed) => format!("{} · {}", phase_label(phase), clock(elapsed)),
                None => phase_label(phase).to_owned(),
            },
            (None, _) => phase_label(phase).to_owned(),
        };
        let action = if self.session.id.is_none() {
            Button::new("start", t("开始串流", "Start streaming"))
                .icon(IconName::Play)
                .primary()
                .size(ControlSize::Lg)
                .disabled(
                    self.devices_locked()
                        || !self.backend_available
                        || self.source().is_none()
                        || self.receiver().is_none(),
                )
                .on_click(click(this, |player, window, cx| {
                    player.start(None, window, cx)
                }))
        } else if phase == Phase::Stopping {
            Button::new("stop", t("停止中…", "Stopping…"))
                .size(ControlSize::Lg)
                .loading(true)
        } else if self.connected() {
            Button::new("stop", t("停止串流", "Stop streaming"))
                .icon(IconName::Square)
                .variant(ButtonVariant::Danger)
                .size(ControlSize::Lg)
                .on_click(click(this, |player, _, cx| player.stop(cx)))
        } else {
            Button::new("stop", t("取消连接", "Cancel"))
                .size(ControlSize::Lg)
                .on_click(click(this, |player, _, cx| player.stop(cx)))
        };
        let level = self.volume_level();
        let muted = self.session.volume.confirmed == Some(0.);
        // 悬停 1 秒后才显示音量百分比，平时不占位置。
        // The percentage shows only after a one-second hover, so it takes no room otherwise.
        let tip: SharedString = if playing {
            tf!("音量 {}%", "Volume {}%", level.round() as u32).into()
        } else {
            t("连接后可调节音量", "Volume is available once connected").into()
        };
        let slider = TooltipTrigger::new(
            "volume-tip",
            Slider::new("volume", level)
                .range(0., 100.)
                .step(1.)
                .disabled(!playing)
                .on_change(edit(this, |player, level: f64, window, cx| {
                    player.schedule_volume(level, window, cx)
                })),
            move |_, cx| small(tip.clone(), cx).into_any_element(),
        )
        .delay(Duration::from_secs(1));
        let volume = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .w(px(340.))
            .child(
                IconButton::new(
                    "mute",
                    if muted {
                        IconName::VolumeX
                    } else {
                        IconName::Volume2
                    },
                )
                .variant(ButtonVariant::Ghost)
                .tooltip(if muted {
                    t("取消静音", "Unmute")
                } else {
                    t("静音", "Mute")
                })
                .disabled(!playing)
                .on_click(click(this, |player, _, cx| player.toggle_mute(cx))),
            )
            .child(div().flex_1().child(slider))
            // 视觉居中：右侧留出与静音图标等宽的空位，居中的是音量条本身。
            // Optical centering: a spacer as wide as the mute icon makes the slider itself centered.
            .child(div().size(px(32.)).flex_none());
        let summary = div()
            .flex_1()
            .flex_basis(px(0.))
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(theme.text_size(TextSize::Sm))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(target),
            )
            .child(
                div()
                    .flex()
                    .child(Badge::new(status).tone(phase_tone(phase)).dot()),
            );
        div()
            .flex_none()
            .border_t_1()
            .border_color(c.border)
            .bg(c.surface)
            .child(
                page().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .h(px(72.))
                        .px(INSET)
                        .child(summary)
                        .child(volume)
                        .child(
                            div()
                                .flex_1()
                                .flex_basis(px(0.))
                                .flex()
                                .justify_end()
                                .child(action),
                        ),
                ),
            )
    }
}

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let bar = drag_region("titlebar", window, cx);
        let app: &App = cx;
        let theme = app.theme();
        let c = &theme.colors;
        let body = match self.page {
            Page::Playback => self.playback(&this, window, app).into_any_element(),
            Page::General => self.general(&this, app).into_any_element(),
            Page::Runtime => self.runtime(app).into_any_element(),
            Page::Technical => self.technical(&this, app).into_any_element(),
            Page::Logs => self.logs(&this, app).into_any_element(),
        };
        let error: Option<SharedString> = self
            .message
            .clone()
            .or_else(|| self.session.error.clone())
            .map(Into::into);
        let dismiss = this.clone();
        let settings = self.page != Page::Playback;
        // 设置页换成下沉底色，卡片浮在上面；主页面保持原底色。
        // Settings sit on the sunken tone with cards above it; the main page keeps its own.
        // 动画作用在内容列里面，不会影响列的居中。
        // Animate inside the column, so its centering is never disturbed.
        let column = div().child(body);
        let content = if settings {
            page()
                .pt(GUTTER)
                .pb(GUTTER)
                .child(fade_in(column, ("page", self.page as usize), app))
        } else {
            page()
                .pb(GUTTER)
                .child(arrive(column, ("page", self.page as usize), app))
        };
        if self.source_open && (self.page != Page::Playback || self.locked()) {
            self.source_open = false;
        }
        if self.source_open {
            let shift = if cx.theme().reduced_motion {
                self.source_scroll.still()
            } else {
                let (shift, moving) = self.source_scroll.frame();
                if moving {
                    window.request_animation_frame();
                }
                shift
            };
            self.source_shift.set(shift);
        }
        if self.scroll.page != self.page as usize {
            self.scroll.reset();
            self.scroll.page = self.page as usize;
        }
        let shift = if theme.reduced_motion {
            self.scroll.still()
        } else {
            let (shift, moving) = self.scroll.frame();
            if moving {
                window.request_animation_frame();
            }
            shift
        };

        let wheel = this.clone();
        // 底部播放栏之上的所有内容是一块平面；进入设置时整块平移。
        // Everything above the dock is one plane; entering settings moves it as a whole.
        let plane = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.header(bar, &this, app))
            .when(settings, |el| el.child(self.tabs(&this, app)))
            .when_some(error, |el, message| {
                let id = SharedString::from(format!("error-{message}"));
                el.child(arrive(
                    page().pb_3().when(settings, |el| el.pt_3()).child(
                        Banner::new("error", Severity::Danger, message).on_dismiss(move |_, cx| {
                            let _ = dismiss.update(cx, |player, cx| {
                                player.message = None;
                                player.session.error = None;
                                cx.notify();
                            });
                        }),
                    ),
                    id,
                    app,
                ))
            })
            .child(
                div()
                    .id("content")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    // 滚动由 elastic.rs 自己管理，这里只裁剪。
                    // elastic.rs does the scrolling; this only clips.
                    .overflow_hidden()
                    .when(settings, |el| el.bg(c.sunken))
                    .child(measure(self.scroll.viewport.clone()))
                    .on_scroll(move |event, _, cx| {
                        let dy = f32::from(event.delta.pixel_delta(px(20.)).y);
                        let _ = wheel.update(cx, |player, cx| {
                            player.scroll.wheel(dy);
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .relative()
                            .top(shift)
                            .child(content)
                            .child(measure(self.scroll.content.clone())),
                    ),
            );
        let plane = if settings {
            plane_in(plane, app).into_any_element()
        } else {
            plane.into_any_element()
        };
        FocusScope::new(&self.focus)
            .root()
            .size_full()
            .flex()
            .flex_col()
            .bg(c.bg)
            .text_color(c.fg)
            .text_size(theme.text_size(TextSize::Base))
            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
            .child(plane)
            .when_some(self.prompt_name(), |el, name| {
                el.child(arrive(
                    page().child(self.password_prompt(name, &this, app)),
                    "password",
                    app,
                ))
            })
            .child(self.dock(&this, app))
    }
}

/// 等宽代码块：日志、报告和设备属性。超过高度后在块内滚动。
/// Monospace block for logs, reports and device properties; scrolls inside itself when tall.
pub(crate) fn mono(
    id: &'static str,
    lines: impl IntoIterator<Item = impl Into<SharedString>>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let c = &theme.colors;
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_0p5()
        .p_3()
        .max_h(px(240.))
        .overflow_y_scroll()
        .rounded(theme.radius(Radius::Md))
        .bg(c.sunken)
        .font_family("JetBrains Mono")
        .text_size(theme.text_size(TextSize::Xs))
        .text_color(c.fg_muted)
        .children(lines.into_iter().map(|line| div().child(line.into())))
}
