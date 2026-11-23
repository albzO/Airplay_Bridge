//! 分组与网格常量：组标题在卡片外，卡片圆角与内部控件同心。
//! Groups and grid constants: the title sits outside the card, whose radius is concentric with its controls.
use ely_core::theme::{ActiveTheme, TextSize};
use gpui::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce, SharedString,
    Styled, Window, div, prelude::*, px,
};
use smallvec::SmallVec;

/// 8px 网格：页面外边距 32，组间 24，组标题与卡片 8，卡片内边距 16。
/// 8px grid: 32 page gutter, 24 between groups, 8 between a group's title and card, 16 card inset.
pub const GUTTER: Pixels = px(32.);
/// 内容列最大宽度（含两侧边距）；窗口更宽时内容居中，不再拉长。
/// Maximum width of the content column, gutters included; wider windows center it.
pub const PAGE_MAX: Pixels = px(960.);
pub const GAP: Pixels = px(24.);
pub const TIGHT: Pixels = px(8.);
pub const INSET: Pixels = px(16.);
/// 卡片内每一行上下各留的空白；首尾行到卡片边与行到分隔线相同，行内容在上下边界之间居中。
/// Space above and below every row in a card. The first and last rows get the same space to the
/// card edge as rows get to a divider, so each row sits centered between its boundaries.
pub const ROW_PAD: Pixels = px(12.);
/// Ely 控件（输入框、下拉、按钮）的圆角；内部卡片与之相同。
/// Corner radius of Ely controls (inputs, selects, buttons); inner cards match it.
pub const CONTROL_RADIUS: Pixels = px(6.);
/// 卡片圆角：内部控件圆角的两倍。严格同心需要 6 + 16 = 22，在这个窗口尺寸下显得过圆。
/// Card radius: twice the control radius. Strictly concentric would be 6 + 16 = 22, which reads
/// as too round at this window size.
pub const PANEL_RADIUS: Pixels = px(12.);

#[derive(IntoElement)]
pub struct CardHeader {
    title: SharedString,
    description: Option<SharedString>,
    action: Option<AnyElement>,
}

impl CardHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            action: None,
        }
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for CardHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        // 标题与卡片内文字左缘对齐；用小号、次要色与行内文字区分。
        // The title lines up with text inside the card; small and muted so it reads as a label.
        div()
            .flex()
            .items_end()
            .justify_between()
            .gap_4()
            .min_h(px(28.))
            .px(INSET)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(theme.text_size(TextSize::Sm))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.colors.fg_muted)
                            .child(self.title),
                    )
                    .children(self.description.map(|text| {
                        div()
                            .text_size(theme.text_size(TextSize::Xs))
                            .text_color(theme.colors.fg_subtle)
                            .child(text)
                    })),
            )
            .children(self.action)
    }
}

/// 分组：标题在外，内容在带背景的卡片里；卡片在子元素之间画分隔线。
/// A group: title outside, content in a filled card that draws hairlines between its children.
#[derive(IntoElement)]
pub struct Card {
    header: Option<CardHeader>,
    body: SmallVec<[AnyElement; 4]>,
}

impl Card {
    pub fn new() -> Self {
        Self {
            header: None,
            body: SmallVec::new(),
        }
    }

    pub fn header(mut self, header: CardHeader) -> Self {
        self.header = Some(header);
        self
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.theme().colors;
        let border = c.border;
        let mut card = div()
            .flex()
            .flex_col()
            .px(INSET)
            .rounded(PANEL_RADIUS)
            .border_1()
            .border_color(border)
            .bg(c.surface);
        for (index, child) in self.body.into_iter().enumerate() {
            if index > 0 {
                card = card.child(div().h(px(1.)).bg(border));
            }
            card = card.child(div().py(ROW_PAD).child(child));
        }
        div()
            .flex()
            .flex_col()
            .gap(TIGHT)
            .children(self.header)
            .child(card)
    }
}

/// 只读数据行：名称在左，数值贴右。
/// Read-only data rows: name on the left, value flush right.
#[derive(IntoElement)]
pub struct Rows {
    items: Vec<(SharedString, SharedString)>,
}

impl Rows {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn text(mut self, label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        self.items.push((label.into(), value.into()));
        self
    }
}

impl RenderOnce for Rows {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let c = &theme.colors;
        let last = self.items.len().saturating_sub(1);
        let rows = self
            .items
            .into_iter()
            .enumerate()
            .map(move |(index, (label, value))| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_6()
                    .when(index > 0, |row| {
                        row.pt(ROW_PAD).border_t_1().border_color(c.border)
                    })
                    .when(index < last, |row| row.pb(ROW_PAD))
                    .text_size(theme.text_size(TextSize::Sm))
                    // 名称与设置项标题同样式：白色、加粗；数值保持常规字重。
                    // Labels match setting titles, full colour and medium weight; values stay regular.
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(c.fg)
                            .child(label),
                    )
                    .child(div().min_w_0().text_right().text_color(c.fg).child(value))
            });
        div().flex().flex_col().children(rows)
    }
}
