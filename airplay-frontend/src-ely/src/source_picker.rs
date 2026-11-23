//! 音频来源下拉。外观沿用 Ely 的选择框与浮层，列表滚动换成 elastic.rs 的平滑与弹性滚动。
//! The audio source picker. It keeps Ely's select box and popup look, but its list scrolls with
//! elastic.rs: smooth, with elastic edges.
use crate::{
    lang::t,
    player::Player,
    view::{click, measure},
};
use ely_core::{
    forms::{Choice, float_height, marked_row, surface},
    primitives::{Icon, IconName},
    theme::{ActiveTheme, ControlSize, IconSize, Radius, TextSize},
};
use gpui::{
    App, Bounds, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    StatefulInteractiveElement, Styled, WeakEntity, Window, canvas, div, prelude::*, px,
};
use homepod_test::capture::Input;
use std::{cell::Cell, rc::Rc};

fn choice(input: &Input) -> Choice {
    let playback = input.flow == "playback";
    let choice = Choice::new(input.id.clone(), input.name.clone()).icon(if playback {
        IconName::Volume2
    } else {
        IconName::Mic
    });
    if input.channels.unwrap_or(0) == 0 {
        choice.disabled()
    } else {
        choice
    }
}

impl Player {
    pub(crate) fn source_picker(
        &self,
        this: &WeakEntity<Self>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let disabled = self.locked() || self.inputs.is_empty();
        let open = self.source_open && !disabled;
        let source = self.source();
        let anchor = self.source_anchor.clone();
        let trigger = div()
            .id("source-trigger")
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .h(theme.control_height(ControlSize::Lg))
            .px(theme.control_padding(ControlSize::Lg))
            .rounded(theme.radius(Radius::Md))
            .border_1()
            .border_color(if open { c.focus } else { c.border_strong })
            .bg(if disabled { c.sunken } else { c.surface })
            .text_size(theme.text_size(TextSize::Base))
            .when(!disabled, |el| el.cursor_pointer())
            .when_some(source, |el, source| {
                el.child(
                    Icon::new(if source.flow == "playback" {
                        IconName::Volume2
                    } else {
                        IconName::Mic
                    })
                    .size(IconSize::Sm)
                    .color(c.fg_muted),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(match (source, disabled) {
                        (_, true) => c.fg_disabled,
                        (Some(_), false) => c.fg,
                        (None, false) => c.fg_subtle,
                    })
                    .child(source.map_or_else(
                        || t("请选择音频流来源", "Choose an audio source").to_owned(),
                        |source| source.name.clone(),
                    )),
            )
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Xs)
                    .color(c.fg_subtle),
            )
            // 记下下拉框的位置，浮层贴着它展开。
            // Record the box's bounds so the popup opens against it.
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            );
        if disabled {
            return trigger.into_any_element();
        }
        let toggle = this.clone();
        let trigger = trigger.on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let _ = toggle.update(cx, |player, cx| {
                player.source_open = !player.source_open;
                if player.source_open {
                    player.source_scroll.reset();
                }
                cx.notify();
            });
        });
        if !open {
            return trigger.into_any_element();
        }
        trigger
            .child(self.source_list(this, window, cx))
            .into_any_element()
    }

    fn source_list(&self, this: &WeakEntity<Self>, window: &Window, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let anchor: Bounds<Pixels> = self.source_anchor.get();
        let selected = self.settings.endpoint.clone();
        let header = |text: &'static str| {
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_size(theme.text_size(TextSize::Xs))
                .text_color(c.fg_subtle)
                .child(text)
        };
        let row = |index: usize, input: &Input| {
            let id = input.id.clone();
            let enabled = input.channels.unwrap_or(0) > 0;
            marked_row(
                ("source-option", index),
                &choice(input),
                None,
                false,
                Some(input.id == selected),
                cx,
            )
            .when(enabled, |row| {
                row.on_click(click(this, move |player, window, cx| {
                    player.source_open = false;
                    player.select_source(&id, window, cx);
                    cx.notify();
                }))
            })
        };
        let playback: Vec<_> = self
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.flow == "playback")
            .map(|(index, input)| row(index, input))
            .collect();
        let recording: Vec<_> = self
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.flow != "playback")
            .map(|(index, input)| row(index, input))
            .collect();
        let rows = self.inputs.len() + 2;
        let height = (theme
            .control_height(ControlSize::Md)
            .to_pixels(window.rem_size())
            * rows as f32)
            .min(theme.list_max_height().to_pixels(window.rem_size()));
        let wheel = this.clone();
        let close = this.clone();
        let list = surface("source-list", cx)
            .w(anchor.size.width)
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                let _ = close.update(cx, |player, cx| {
                    player.source_open = false;
                    cx.notify();
                });
            })
            .child(
                div()
                    .id("source-viewport")
                    .relative()
                    .max_h(theme.list_max_height())
                    .overflow_hidden()
                    .child(measure(self.source_scroll.viewport.clone()))
                    .on_scroll(move |event, _, cx| {
                        let dy = f32::from(event.delta.pixel_delta(px(20.)).y);
                        let _ = wheel.update(cx, |player, cx| {
                            player.source_scroll.wheel(dy);
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .relative()
                            .top(self.source_shift.get())
                            .p_1()
                            .flex()
                            .flex_col()
                            .child(header(t("播放设备", "Playback devices")))
                            .children(playback)
                            .child(header(t("录音设备", "Recording devices")))
                            .children(recording)
                            .child(measure(self.source_scroll.content.clone())),
                    ),
            );
        float_height("source", anchor, height, list, window, cx)
    }
}

/// 下拉列表本帧的位移由渲染入口写入，列表构建时读取。
/// The list's shift for this frame, written by the render entry and read while building the list.
pub(crate) type Shift = Rc<Cell<Pixels>>;
