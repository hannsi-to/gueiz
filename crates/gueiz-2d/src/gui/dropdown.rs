//! 畳んだ一覧から 1 つ選ぶもの。
//!
//! # 開いた一覧は木の外へ出さない
//!
//! 一覧は自分より大きく、**親の切り抜きを突き抜けて、どの図形よりも手前に**
//! 出なければなりません。素直に考えると「開いたら一覧を根の直下へ移す」
//! ことになりますが、それをやると
//!
//! - 木を触りながら木を組み替えることになる（[`crate::gui::EventContext`] からは触れません）
//! - 移したウィジェットの親が変わるので、札や焦点の付け替えが要る
//! - 閉じたときに元の場所へ戻す始末が付く
//!
//! ので、木は動かしません。代わりに [`Behavior::overlay`] を立てて、
//! **描く周回をもう 1 回もらいます**（[`Widget::paint_overlay`]）。
//!
//! ```text
//! 1 周目: 木をたどって描く        ← 畳んだ見た目はここ。親の切り抜きを受ける
//! 2 周目: 浮かせるものだけ描く     ← 開いた一覧はここ。切り抜き無し・いちばん手前
//! ```
//!
//! 当たり判定も同じ順です。**浮かせているものから先に当てます。**
//! そうしないと、一覧の上を押したのに後ろのボタンが反応します。
//!
//! # 上に出すか下に出すか
//!
//! 画面の下からはみ出すなら上に出します。これを決めるには
//! **自分が画面のどこに居るか**を知る必要があるので、置く周回で
//! [`ArrangeContext::screen_origin`] と [`ArrangeContext::viewport`] を見ます。
//!
//! ウィジェットが自分の画面上の位置を見るのは、ここだけに許した例外です
//! （ふつうは置き場所で形が変わってはいけません）。
//!
//! # 一覧は項目の単位で送る
//!
//! [`Dropdown::max_visible`] を超えると、車輪で送れます。送るのは
//! **項目 1 つぶんずつ**です。画素単位にすると、半分だけ見えている行の
//! 当たり判定を別に用意することになるので、それは
//! [`crate::gui::scroll_area`] の仕事にしています。
//!
//! # 見た目
//!
//! | 役 | 何に使うか |
//! |---|---|
//! | [`Dropdown::role`] | 畳んだ見た目（背景・枠線・字） |
//! | [`Dropdown::popup_role`] | 開いた一覧の下敷き |
//! | [`Dropdown::item_role`] | 項目 1 つ。`Hovered` で指している行、`Pressed` で選ばれている行 |
//!
//! 指している行と選ばれている行を [`crate::gui::theme::StateKey`] で分けているので、
//! 色は表だけで決められます。
//!
//! # 使い方
//!
//! ```no_run
//! # use gueiz_2d::gui::dropdown::Dropdown;
//! # use gueiz_2d::gui::id::Tag;
//! # use gueiz_2d::gui::widget::ActionKind;
//! # use gueiz_2d::gui::Gui;
//! # fn build(gui: &mut Gui, parent: gueiz_2d::gui::id::WidgetId) {
//! const QUALITY: Tag = Tag::new("quality");
//!
//! let quality = gui.tree_mut().add_child(
//!     parent,
//!     Box::new(
//!         Dropdown::new(["低", "中", "高"])
//!             .selected(1)
//!             .placeholder("選んでください"),
//!     ),
//! );
//! gui.tree_mut().set_tag(quality, QUALITY);
//!
//! for action in gui.drain_actions() {
//!     if action.is(QUALITY) && action.kind == ActionKind::ValueChanged {
//!         let chosen = gui
//!             .tree()
//!             .get_as::<Dropdown>(quality)
//!             .and_then(Dropdown::selected_item);
//!
//!         println!("{chosen:?}");
//!     }
//! }
//! # }
//! ```
//!
//! [`ArrangeContext::screen_origin`]: crate::gui::ArrangeContext::screen_origin
//! [`ArrangeContext::viewport`]: crate::gui::ArrangeContext::viewport
//! [`Behavior::overlay`]: crate::gui::widget::Behavior::overlay

use crate::gui::event::{Event, EventResult, Key, NamedKey, PointerButton};
use crate::gui::geometry::{Align, Corners, Point, Rect, Size};
use crate::gui::layout::Constraints;
use crate::gui::painter::{Painter, TextLayoutOptions};
use crate::gui::theme::Role;
use crate::gui::widget::{ActionKind, Behavior, WidgetState};
use crate::gui::{ArrangeContext, EventContext, MeasureContext, PaintContext, Widget};

/// 畳んだ見た目の右に出す印のための幅。
const MARKER_WIDTH: f32 = 20.0;

/// 印の大きさ。畳んだ高さに対する割合。
const MARKER_SCALE: f32 = 0.22;

/// 一覧と畳んだ見た目のあいだ。
const POPUP_GAP: f32 = 2.0;

/// 畳んだ一覧から 1 つ選ぶもの。
pub struct Dropdown {
    items: Vec<String>,
    selected: Option<usize>,
    open: bool,

    /// 指している行。開いているあいだだけ意味がある。
    highlighted: usize,
    /// 一覧のいちばん上に出す行。
    first_visible: usize,

    placeholder: String,
    role: Role,
    popup_role: Role,
    item_role: Role,

    /// 一覧に一度に出す行数。
    max_visible: usize,
    /// 行の高さ。`None` なら畳んだ高さと同じ。
    item_height: Option<f32>,
    min_width: f32,

    // --- 置く周回が埋める ---
    size: Size,
    /// 一覧を上に出すか。画面の下に収まらないとき立つ。
    above: bool,
    /// 解いた行の高さ。
    resolved_item_height: f32,
    /// 解いた余白と字の大きさ。描くときと当てるときで同じ値を使う。
    text_size: f32,
}

impl Dropdown {
    pub fn new<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            selected: None,
            open: false,
            highlighted: 0,
            first_visible: 0,
            placeholder: String::new(),
            role: Role::CONTROL,
            popup_role: Role::SURFACE,
            item_role: Role::CONTROL,
            max_visible: 8,
            item_height: None,
            min_width: 120.0,
            size: Size::ZERO,
            above: false,
            resolved_item_height: 0.0,
            text_size: 16.0,
        }
    }

    /// 最初から選んでおく。範囲外なら何も選ばれません。
    pub fn selected(mut self, index: usize) -> Self {
        if index < self.items.len() {
            self.selected = Some(index);
            self.highlighted = index;
        }

        self
    }

    /// 何も選ばれていないときに出す字。
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// 開いた一覧の下敷きの役。
    pub fn popup_role(mut self, popup_role: Role) -> Self {
        self.popup_role = popup_role;
        self
    }

    /// 項目 1 つの役。`Hovered` が指している行、`Pressed` が選ばれている行。
    pub fn item_role(mut self, item_role: Role) -> Self {
        self.item_role = item_role;
        self
    }

    /// 一覧に一度に出す行数。超えたぶんは車輪で送ります。
    pub fn max_visible(mut self, max_visible: usize) -> Self {
        self.max_visible = max_visible.max(1);
        self
    }

    /// 行の高さ。既定は畳んだ高さと同じ。
    pub fn item_height(mut self, item_height: f32) -> Self {
        self.item_height = Some(item_height.max(1.0));
        self
    }

    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width.max(0.0);
        self
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// 選ばれている番号。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 選ばれている字。
    pub fn selected_item(&self) -> Option<&str> {
        self.selected.and_then(|index| self.items.get(index)).map(String::as_str)
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 指している行。
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// いちばん上に出している行。
    pub fn first_visible(&self) -> usize {
        self.first_visible
    }

    /// 選び直す。**動いたら `true`。** 範囲外は無視します。
    ///
    /// 呼ぶ側から変えたときは
    /// [`WidgetTree::request_layout`](crate::gui::tree::WidgetTree::request_layout)
    /// も呼んでください。
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.items.len() || self.selected == Some(index) {
            return false;
        }

        self.selected = Some(index);
        self.highlighted = index;

        true
    }

    /// 何も選ばれていない状態に戻す。
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// 項目を入れ替える。選んでいたものは外れます。
    pub fn set_items<I, S>(&mut self, items: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.items = items.into_iter().map(Into::into).collect();
        self.selected = None;
        self.highlighted = 0;
        self.first_visible = 0;
        self.open = false;
    }

    /// 開く。項目が無ければ開きません。
    pub fn open(&mut self) -> bool {
        if self.open || self.items.is_empty() {
            return false;
        }

        self.open = true;
        self.highlighted = self.selected.unwrap_or(0);
        self.reveal_highlighted();

        true
    }

    /// 閉じる。
    pub fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }

        self.open = false;

        true
    }

    /// 一度に出す行数。項目が少なければそのぶん。
    pub fn visible_count(&self) -> usize {
        self.items.len().min(self.max_visible)
    }

    /// 開いた一覧の矩形。**局所座標**（自分の左上から）。閉じていれば空。
    pub fn popup_rect(&self) -> Rect {
        if !self.open || self.items.is_empty() {
            return Rect::ZERO;
        }

        let height = self.visible_count() as f32 * self.resolved_item_height;

        let y = if self.above {
            -(height + POPUP_GAP)
        } else {
            self.size.height + POPUP_GAP
        };

        Rect::new(0.0, y, self.size.width, height)
    }

    /// その行の矩形。局所座標。見えていなければ `None`。
    pub fn item_rect(&self, index: usize) -> Option<Rect> {
        let popup = self.popup_rect();

        if popup.is_empty() {
            return None;
        }

        let row = index.checked_sub(self.first_visible)?;

        if row >= self.visible_count() {
            return None;
        }

        Some(Rect::new(
            popup.x,
            popup.y + row as f32 * self.resolved_item_height,
            popup.width,
            self.resolved_item_height,
        ))
    }

    /// その点にある行。局所座標。
    pub fn item_at(&self, local: Point) -> Option<usize> {
        let popup = self.popup_rect();

        if !popup.contains(local) {
            return None;
        }

        let row = ((local.y - popup.y) / self.resolved_item_height).floor() as usize;
        let index = self.first_visible + row;

        (index < self.items.len() && row < self.visible_count()).then_some(index)
    }

    /// 指している行が見えるところまで送る。
    fn reveal_highlighted(&mut self) {
        let visible = self.visible_count();

        if self.highlighted < self.first_visible {
            self.first_visible = self.highlighted;
        } else if self.highlighted >= self.first_visible + visible {
            self.first_visible = self.highlighted + 1 - visible;
        }

        // 下に余白が出ないところまで戻す。
        let max = self.items.len().saturating_sub(visible);
        self.first_visible = self.first_visible.min(max);
    }

    /// 指す行を 1 つ動かす。端で止まります。
    fn move_highlight(&mut self, forward: bool) {
        if self.items.is_empty() {
            return;
        }

        self.highlighted = if forward {
            (self.highlighted + 1).min(self.items.len() - 1)
        } else {
            self.highlighted.saturating_sub(1)
        };

        self.reveal_highlighted();
    }

    /// 一覧を送る。動いたら `true`。
    fn scroll_items(&mut self, forward: bool) -> bool {
        let max = self.items.len().saturating_sub(self.visible_count());

        let next = if forward {
            (self.first_visible + 1).min(max)
        } else {
            self.first_visible.saturating_sub(1)
        };

        let moved = next != self.first_visible;
        self.first_visible = next;

        moved
    }

    /// 項目 1 つの見た目を引くときの状態。
    ///
    /// 指している行を `Hovered`、選ばれている行を `Pressed` に写す。
    /// 色を 2 つに分けるのに、新しい仕組みを増やさずに済む。
    fn item_state(&self, index: usize, state: WidgetState) -> WidgetState {
        WidgetState {
            hovered: self.open && index == self.highlighted,
            pressed: self.selected == Some(index),
            focused: false,
            disabled: state.disabled,
        }
    }

    fn text_options(&self) -> TextLayoutOptions {
        TextLayoutOptions {
            size: self.text_size,
            horizontal: Align::Start,
            vertical: Align::Center,
            wrap: false,
        }
    }

    /// 畳んだ見た目に出す字。
    fn closed_label(&self) -> &str {
        self.selected_item().unwrap_or(&self.placeholder)
    }
}

impl Widget for Dropdown {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let style = context.theme().style(self.role, context.state());
        self.text_size = style.text_size;

        let options = self.text_options();

        // いちばん長い項目に合わせる。開いたときに字が切れないように。
        let widest = self
            .items
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(self.placeholder.as_str()))
            .map(|item| context.measure_text(item, options, None).width)
            .fold(0.0_f32, f32::max);

        let height = (context.line_height(style.text_size) + style.padding.vertical())
            .max(context.metrics().control_height);

        let width = (widest + MARKER_WIDTH + style.padding.horizontal()).max(self.min_width);

        constraints.constrain(Size::new(width, height))
    }

    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        self.size = bounds.size();
        self.resolved_item_height = self.item_height.unwrap_or(self.size.height);

        // 下に収まらないなら上に出す。**ここだけは画面上の位置を見る。**
        let height = self.visible_count() as f32 * self.resolved_item_height + POPUP_GAP;
        let below = context.screen_origin().y + self.size.height + height;
        let above_room = context.screen_origin().y - height;

        // 下が駄目で、上に place があるときだけひっくり返す。
        // どちらも駄目なら下。切れるのは仕方がないが、上に出すと
        // 画面の外に頭が出て、もっと見えなくなる。
        self.above = below > context.viewport().height && above_room >= 0.0;

        self.reveal_highlighted();
    }

    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let state = context.state();
        let style = context.theme().style(self.role, state);
        let frame = context.rect();

        painter.rounded_rect(frame, style.corners, style.background);

        if style.border_width > 0.0 {
            painter.border(frame, style.corners, style.border_width, style.border);
        }

        let inner = frame.deflate(style.padding);

        // 字は印のぶんを空けて置く。
        let label = self.closed_label();

        if !label.is_empty() {
            painter.text(
                Rect::new(
                    inner.x,
                    inner.y,
                    (inner.width - MARKER_WIDTH).max(0.0),
                    inner.height,
                ),
                label,
                self.text_options(),
                style.foreground,
            );
        }

        // 開いている向きを指す三角。
        let marker = frame.height * MARKER_SCALE;
        let centre = Point::new(inner.right() - MARKER_WIDTH / 2.0, frame.center().y);

        let (tip, base) = if self.open && !self.above {
            // 開いていて下に出ているなら、閉じる向き（上）を指す。
            (centre.y - marker / 2.0, centre.y + marker / 2.0)
        } else {
            (centre.y + marker / 2.0, centre.y - marker / 2.0)
        };

        painter.polyline(
            &[
                Point::new(centre.x - marker, base),
                Point::new(centre.x + marker, base),
                Point::new(centre.x, tip),
            ],
            1.0,
            style.foreground,
            true,
        );
    }

    /// 開いた一覧。**切り抜きを突き抜けて、いちばん手前に出る。**
    fn paint_overlay(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let popup = self.popup_rect();

        if popup.is_empty() {
            return;
        }

        let state = context.state();
        let shell = context.theme().style(self.popup_role, state);

        painter.rounded_rect(popup, shell.corners, shell.background);

        // 行は下敷きの角から出ないように切る。
        painter.with_clip(popup, shell.corners, |painter| {
            let visible = self.visible_count();

            for row in 0..visible {
                let index = self.first_visible + row;

                let Some(rect) = self.item_rect(index) else {
                    continue;
                };

                let item = context
                    .theme()
                    .style(self.item_role, self.item_state(index, state));

                painter.rect(rect, item.background);

                painter.text(
                    rect.deflate(item.padding),
                    self.items[index].as_str(),
                    self.text_options(),
                    item.foreground,
                );
            }
        });

        if shell.border_width > 0.0 {
            painter.border(popup, shell.corners, shell.border_width, shell.border);
        }
    }

    fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
        match event {
            Event::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                // 開いている一覧の上。選んで閉じる。
                if let Some(index) = self.item_at(*position) {
                    let changed = self.select(index);
                    self.close();

                    if changed {
                        context.emit(ActionKind::ValueChanged);
                    }

                    context.request_layout();

                    return EventResult::Consumed;
                }

                // 畳んだ見た目の上。開け閉めする。
                if context.rect().contains(*position) {
                    if self.open {
                        self.close();
                    } else {
                        self.open();
                    }

                    context.request_layout();

                    return EventResult::Consumed;
                }

                // どちらでもない。開いていたら閉じるだけにして、
                // **押したことは外へ渡す**（後ろのものも反応してよい）。
                if self.close() {
                    context.request_layout();
                }

                EventResult::Ignored
            }

            Event::PointerMove { position, .. } if self.open => {
                // 指す行を追いかける。
                if let Some(index) = self.item_at(*position) {
                    if self.highlighted != index {
                        self.highlighted = index;
                    }

                    return EventResult::Consumed;
                }

                EventResult::Ignored
            }

            Event::Scrolled { delta, .. } if self.open => {
                let amount = delta.to_pixels(1.0);

                // 画素ではなく行で送る。半端に見えている行を作らない。
                if self.scroll_items(amount.y < 0.0) {
                    return EventResult::Consumed;
                }

                // 端。外へ渡す。
                EventResult::Ignored
            }

            Event::KeyPressed { key, .. } => self.on_key(context, key),

            // 焦点が外れたら閉じる。開いたまま残ると、他を押せなくなる。
            Event::FocusLost => {
                if self.close() {
                    context.request_layout();
                }

                EventResult::Ignored
            }

            // 押されたことは受け取る。後ろへ抜けないように。
            event if event.is_pointer() => EventResult::Consumed,

            _ => EventResult::Ignored,
        }
    }

    /// 畳んだ矩形か、開いた一覧の上なら当たり。
    ///
    /// 一覧は自分の矩形の外にあるので、矩形だけで見ると当たりません。
    fn hit_test(&self, local: Point, size: Size) -> bool {
        if Rect::from_origin_size(Point::ZERO, size).contains(local) {
            return true;
        }

        self.popup_rect().contains(local)
    }

    fn clip_corners(&self) -> Corners {
        Corners::ZERO
    }

    fn behavior(&self) -> Behavior {
        // 開いているあいだだけ浮かせる。閉じていれば普通のボタン。
        Behavior::CONTROL.overlay(self.open)
    }
}

impl Dropdown {
    fn on_key(&mut self, context: &mut EventContext<'_>, key: &Key) -> EventResult {
        let Key::Named(named) = key else {
            return EventResult::Ignored;
        };

        match named {
            NamedKey::Space | NamedKey::Enter => {
                if self.open {
                    // 指している行で決める。
                    let changed = self.select(self.highlighted);
                    self.close();

                    if changed {
                        context.emit(ActionKind::ValueChanged);
                    }
                } else {
                    self.open();
                }

                context.request_layout();

                EventResult::Consumed
            }

            NamedKey::Escape if self.open => {
                self.close();
                context.emit(ActionKind::Cancelled);
                context.request_layout();

                EventResult::Consumed
            }

            NamedKey::ArrowDown | NamedKey::ArrowUp => {
                let forward = matches!(named, NamedKey::ArrowDown);

                if self.open {
                    self.move_highlight(forward);
                    return EventResult::Consumed;
                }

                // 畳んだまま矢印で選び替える。一覧を開かずに済む。
                let next = match self.selected {
                    Some(index) if forward => (index + 1).min(self.items.len().saturating_sub(1)),
                    Some(index) => index.saturating_sub(1),
                    None => 0,
                };

                if self.select(next) {
                    context.emit(ActionKind::ValueChanged);
                    context.request_layout();
                }

                EventResult::Consumed
            }

            NamedKey::Home | NamedKey::End if self.open => {
                self.highlighted = if matches!(named, NamedKey::Home) {
                    0
                } else {
                    self.items.len().saturating_sub(1)
                };

                self.reveal_highlighted();

                EventResult::Consumed
            }

            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Color;
    use crate::gui::container::Stack;
    use crate::gui::context::TextMeasure;
    use crate::gui::event::{InputEvent, ScrollDelta};
    use crate::gui::geometry::Insets;
    use crate::gui::id::WidgetId;
    use crate::gui::painter::Primitive;
    use crate::gui::scroll_area::ScrollArea;
    use crate::gui::theme::{Metrics, StateKey, Style, Theme};
    use crate::gui::Gui;

    struct Monospace;

    impl TextMeasure for Monospace {
        fn measure_text(&self, text: &str, options: TextLayoutOptions, _: Option<f32>) -> Size {
            Size::new(
                text.chars().count() as f32 * options.size * 0.5,
                options.size,
            )
        }

        fn line_height(&self, text_size: f32) -> f32 {
            text_size
        }
    }

    const VIEWPORT: Size = Size {
        width: 400.0,
        height: 300.0,
    };

    /// 畳んだ高さ。`control_height` で決まる。
    const ROW: f32 = 24.0;

    fn theme() -> Theme {
        let mut theme = Theme::new();

        theme.set_metrics(Metrics {
            spacing: 4.0,
            text_size: 16.0,
            control_height: ROW,
            line_height_factor: 1.0,
        });

        theme.set(
            Role::CONTROL,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x202028))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::all(2.0)),
        );
        // 指している行。
        theme.set(
            Role::CONTROL,
            StateKey::Hovered,
            Style::BARE
                .background(Color::hex(0x3355aa))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::all(2.0)),
        );
        // 選ばれている行。
        theme.set(
            Role::CONTROL,
            StateKey::Pressed,
            Style::BARE
                .background(Color::hex(0x114422))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::all(2.0)),
        );
        theme.set(
            Role::SURFACE,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x101014))
                .border(Color::hex(0x444455), 1.0),
        );

        theme
    }

    fn with_dropdown(dropdown: Dropdown) -> (Gui, WidgetId) {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let id = gui.tree_mut().add_child(root, Box::new(dropdown));

        (gui, id)
    }

    fn laid_out(gui: &mut Gui) {
        gui.layout(VIEWPORT, &Monospace);
    }

    fn drop_down(gui: &Gui, id: WidgetId) -> &Dropdown {
        gui.tree().get_as::<Dropdown>(id).expect("Dropdown")
    }

    fn press_at(gui: &mut Gui, at: Point) {
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });
    }

    /// その鍵の畳んだ見た目の真ん中を押す。
    fn press_center(gui: &mut Gui, id: WidgetId) {
        let at = gui.tree().bounds(id).center();
        press_at(gui, at);
    }

    fn press_key(gui: &mut Gui, named: NamedKey) {
        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(named),
            repeat: false,
        });
    }

    fn items() -> Vec<String> {
        (0..5).map(|index| format!("item{index}")).collect()
    }

    // --- 開け閉め ---

    #[test]
    fn clicking_opens_and_closes() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        assert!(!drop_down(&gui, id).is_open());

        press_at(&mut gui, bounds.center());
        assert!(drop_down(&gui, id).is_open());

        laid_out(&mut gui);
        press_at(&mut gui, bounds.center());

        assert!(!drop_down(&gui, id).is_open());
    }

    #[test]
    fn an_empty_dropdown_never_opens() {
        let (mut gui, id) = with_dropdown(Dropdown::new(Vec::<String>::new()));
        laid_out(&mut gui);

        press_center(&mut gui, id);

        assert!(!drop_down(&gui, id).is_open());
        assert!(drop_down(&gui, id).popup_rect().is_empty());
    }

    #[test]
    fn losing_focus_closes_it() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        press_center(&mut gui, id);
        assert!(drop_down(&gui, id).is_open());

        gui.set_focus(WidgetId::NONE);

        assert!(!drop_down(&gui, id).is_open());
    }

    // --- 浮かせる ---

    #[test]
    fn the_popup_only_floats_while_open() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        assert!(!gui.tree().get(id).unwrap().behavior().overlay);

        press_center(&mut gui, id);
        laid_out(&mut gui);

        assert!(
            gui.tree().get(id).unwrap().behavior().overlay,
            "開いているあいだだけ浮く"
        );
    }

    #[test]
    fn the_popup_is_painted_last_and_without_a_clip() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        // 背の低い切り抜く箱の中に入れる。畳んだ見た目は切られ、
        // 開いた一覧は箱の下へ突き抜ける。
        let root = gui.set_root(Box::new(Stack::column()));
        let area = gui
            .tree_mut()
            .add_child(root, Box::new(ScrollArea::vertical()));
        let id = gui
            .tree_mut()
            .add_child(area, Box::new(Dropdown::new(items())));

        gui.tree_mut()
            .get_as_mut::<Stack>(root)
            .unwrap()
            .set_length(area, crate::gui::layout::Length::Fixed(ROW + 6.0));

        laid_out(&mut gui);
        press_center(&mut gui, id);
        laid_out(&mut gui);

        // 箱の切り抜き。畳んだ見た目にはこれが掛かる。
        let box_clip = gui.tree().bounds(area);
        let popup = drop_down(&gui, id)
            .popup_rect()
            .translate(gui.tree().bounds(id).origin());

        let list = gui.paint();

        let mut closed = Vec::new();
        let mut floating = Vec::new();

        for (index, command) in list.commands().iter().enumerate() {
            if command.owner != id {
                continue;
            }

            // 箱で切られているなら畳んだぶん。一覧は箱では切られない
            // （切り抜き無しか、一覧自身の切り抜き）。
            match command.clip {
                Some(clip) if clip.rect == box_clip => closed.push(index),
                _ => floating.push(index),
            }
        }

        assert!(!closed.is_empty(), "畳んだぶんは箱で切り抜かれる");
        assert!(!floating.is_empty(), "一覧は箱では切り抜かれない");

        // 浮かせたぶんは、記録のいちばん後ろに並ぶ = いちばん手前。
        let last_closed = *closed.last().unwrap();
        let first_floating = *floating.first().unwrap();

        assert!(
            first_floating > last_closed,
            "一覧はあとに並ぶ: {first_floating} > {last_closed}"
        );

        // 一覧は箱の外へ出ている。切り抜きを突き抜けている証拠。
        assert!(
            popup.bottom() > box_clip.bottom() || popup.y < box_clip.y,
            "popup {popup:?} vs box {box_clip:?}"
        );
    }

    #[test]
    fn the_popup_wins_the_hit_test_over_what_is_behind() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let id = gui
            .tree_mut()
            .add_child(root, Box::new(Dropdown::new(items())));
        // 一覧の下に重なるボタン代わり。
        let behind = gui.tree_mut().add_child(
            root,
            Box::new(crate::gui::button::Button::new("behind")),
        );

        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        let behind_bounds = gui.tree().bounds(behind);

        // 閉じているうちは後ろのものに当たる。
        assert_eq!(gui.hit_test(behind_bounds.center()), behind);

        press_at(&mut gui, bounds.center());
        laid_out(&mut gui);

        // 開いたら、同じ場所が一覧のものになる。
        assert_eq!(
            gui.hit_test(behind_bounds.center()),
            id,
            "一覧が手前なので一覧に当たる"
        );
    }

    #[test]
    fn a_popup_scrolled_out_of_sight_does_not_float() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let area = gui.set_root(Box::new(ScrollArea::vertical()));
        let list = gui.tree_mut().add_child(area, Box::new(Stack::column()));
        let id = gui
            .tree_mut()
            .add_child(list, Box::new(Dropdown::new(items())));

        // 後ろに長い中身を足して、送れるようにする。
        gui.tree_mut()
            .add_child(list, Box::new(Dropdown::new(items())));
        for _ in 0..20 {
            gui.tree_mut()
                .add_child(list, Box::new(crate::gui::button::Button::new("x")));
        }

        laid_out(&mut gui);
        press_center(&mut gui, id);
        laid_out(&mut gui);

        assert!(drop_down(&gui, id).is_open());
        let floating_before = gui.paint().commands_of(id).count();
        assert!(floating_before > 0);

        // 送って画面の外へ出す。
        gui.tree_mut()
            .get_as_mut::<ScrollArea>(area)
            .unwrap()
            .scroll_to_end();
        gui.tree_mut().request_layout(area);
        laid_out(&mut gui);

        assert_eq!(
            gui.paint().commands_of(id).count(),
            0,
            "持ち主が見えないなら、一覧も宙に残らない"
        );
    }

    // --- 上下の向き ---

    #[test]
    fn the_popup_opens_downwards_when_there_is_room() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        press_center(&mut gui, id);
        laid_out(&mut gui);

        let popup = drop_down(&gui, id).popup_rect();

        assert!(popup.y > 0.0, "自分の下に出る: {popup:?}");
        assert_eq!(popup.height, 5.0 * ROW);
    }

    #[test]
    fn the_popup_flips_up_near_the_bottom_of_the_screen() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        // 下端へ寄せる。
        let root = gui.set_root(Box::new(
            Stack::column().main_align(Align::End),
        ));
        let id = gui
            .tree_mut()
            .add_child(root, Box::new(Dropdown::new(items())));

        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        assert!(bounds.bottom() > VIEWPORT.height - ROW - 1.0, "下端に居る");

        press_at(&mut gui, bounds.center());
        laid_out(&mut gui);

        let popup = drop_down(&gui, id).popup_rect();

        assert!(popup.y < 0.0, "上に出る: {popup:?}");
        // 画面の中に収まっている。
        assert!(bounds.y + popup.y >= 0.0);
    }

    // --- 選ぶ ---

    #[test]
    fn clicking_an_item_selects_it_and_closes() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        press_at(&mut gui, bounds.center());
        laid_out(&mut gui);

        // 3 行目の真ん中。局所座標から画面座標に直す。
        let row = drop_down(&gui, id).item_rect(2).expect("3 行目");
        press_at(&mut gui, bounds.origin() + row.center());

        assert_eq!(drop_down(&gui, id).selected_index(), Some(2));
        assert_eq!(drop_down(&gui, id).selected_item(), Some("item2"));
        assert!(!drop_down(&gui, id).is_open(), "選んだら閉じる");

        let actions = gui.drain_actions();
        assert!(
            actions.iter().any(|action| action.kind == ActionKind::ValueChanged),
            "{actions:?}"
        );
    }

    #[test]
    fn choosing_the_same_item_reports_nothing() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).selected(1));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        press_at(&mut gui, bounds.center());
        laid_out(&mut gui);

        let row = drop_down(&gui, id).item_rect(1).expect("2 行目");
        press_at(&mut gui, bounds.origin() + row.center());

        let actions = gui.drain_actions();

        assert!(
            !actions.iter().any(|action| action.kind == ActionKind::ValueChanged),
            "変わっていないので伝えない: {actions:?}"
        );
        assert!(!drop_down(&gui, id).is_open());
    }

    #[test]
    fn clicking_outside_closes_without_choosing() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).selected(0));
        laid_out(&mut gui);

        press_center(&mut gui, id);
        laid_out(&mut gui);

        // 一覧の横。画面の右端。
        press_at(&mut gui, Point::new(VIEWPORT.width - 2.0, 10.0));

        assert!(!drop_down(&gui, id).is_open());
        assert_eq!(drop_down(&gui, id).selected_index(), Some(0), "選び直さない");
    }

    #[test]
    fn hovering_moves_the_highlight() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        press_at(&mut gui, bounds.center());
        laid_out(&mut gui);

        let row = drop_down(&gui, id).item_rect(3).expect("4 行目");
        gui.handle_input(InputEvent::PointerMoved {
            position: bounds.origin() + row.center(),
        });

        assert_eq!(drop_down(&gui, id).highlighted(), 3);
    }

    // --- 鍵盤 ---

    #[test]
    fn space_opens_then_enter_commits_the_highlight() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);
        gui.set_focus(id);

        press_key(&mut gui, NamedKey::Space);
        assert!(drop_down(&gui, id).is_open());
        laid_out(&mut gui);

        press_key(&mut gui, NamedKey::ArrowDown);
        press_key(&mut gui, NamedKey::ArrowDown);
        assert_eq!(drop_down(&gui, id).highlighted(), 2);

        press_key(&mut gui, NamedKey::Enter);

        assert_eq!(drop_down(&gui, id).selected_index(), Some(2));
        assert!(!drop_down(&gui, id).is_open());
    }

    #[test]
    fn escape_closes_without_choosing() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).selected(0));
        laid_out(&mut gui);
        gui.set_focus(id);

        press_key(&mut gui, NamedKey::Space);
        laid_out(&mut gui);
        press_key(&mut gui, NamedKey::ArrowDown);
        press_key(&mut gui, NamedKey::Escape);

        assert!(!drop_down(&gui, id).is_open());
        assert_eq!(drop_down(&gui, id).selected_index(), Some(0));

        let actions = gui.drain_actions();
        assert!(actions.iter().any(|action| action.kind == ActionKind::Cancelled));
    }

    #[test]
    fn arrows_change_the_choice_while_closed() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).selected(1));
        laid_out(&mut gui);
        gui.set_focus(id);

        press_key(&mut gui, NamedKey::ArrowDown);

        assert!(!drop_down(&gui, id).is_open(), "開かずに選び替える");
        assert_eq!(drop_down(&gui, id).selected_index(), Some(2));

        let actions = gui.drain_actions();
        assert!(actions.iter().any(|action| action.kind == ActionKind::ValueChanged));

        // 端では止まる。
        laid_out(&mut gui);
        for _ in 0..10 {
            press_key(&mut gui, NamedKey::ArrowDown);
            laid_out(&mut gui);
        }

        assert_eq!(drop_down(&gui, id).selected_index(), Some(4));
    }

    #[test]
    fn the_highlight_stops_at_both_ends() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);
        gui.set_focus(id);

        press_key(&mut gui, NamedKey::Space);
        laid_out(&mut gui);

        for _ in 0..10 {
            press_key(&mut gui, NamedKey::ArrowUp);
        }
        assert_eq!(drop_down(&gui, id).highlighted(), 0);

        press_key(&mut gui, NamedKey::End);
        assert_eq!(drop_down(&gui, id).highlighted(), 4);

        press_key(&mut gui, NamedKey::Home);
        assert_eq!(drop_down(&gui, id).highlighted(), 0);
    }

    // --- 送る ---

    #[test]
    fn a_long_list_shows_at_most_max_visible_rows() {
        let many: Vec<String> = (0..20).map(|index| format!("row{index}")).collect();
        let (mut gui, id) = with_dropdown(Dropdown::new(many).max_visible(4));

        laid_out(&mut gui);
        press_center(&mut gui, id);
        laid_out(&mut gui);

        let field = drop_down(&gui, id);

        assert_eq!(field.visible_count(), 4);
        assert_eq!(field.popup_rect().height, 4.0 * ROW);
        assert!(field.item_rect(0).is_some());
        assert!(field.item_rect(4).is_none(), "5 行目は出ていない");
    }

    #[test]
    fn the_wheel_scrolls_the_list_by_rows() {
        let many: Vec<String> = (0..20).map(|index| format!("row{index}")).collect();
        let (mut gui, id) = with_dropdown(Dropdown::new(many).max_visible(4));

        laid_out(&mut gui);
        let bounds = gui.tree().bounds(id);
        press_at(&mut gui, bounds.center());
        laid_out(&mut gui);

        let inside = bounds.origin() + drop_down(&gui, id).item_rect(1).unwrap().center();

        gui.handle_input(InputEvent::Scrolled {
            position: inside,
            delta: ScrollDelta::Lines { x: 0.0, y: -1.0 },
        });

        assert_eq!(drop_down(&gui, id).first_visible(), 1);
        assert!(drop_down(&gui, id).item_rect(0).is_none(), "1 行目は隠れた");

        gui.handle_input(InputEvent::Scrolled {
            position: inside,
            delta: ScrollDelta::Lines { x: 0.0, y: 1.0 },
        });

        assert_eq!(drop_down(&gui, id).first_visible(), 0);
    }

    #[test]
    fn moving_the_highlight_scrolls_it_into_view() {
        let many: Vec<String> = (0..20).map(|index| format!("row{index}")).collect();
        let (mut gui, id) = with_dropdown(Dropdown::new(many).max_visible(4));

        laid_out(&mut gui);
        gui.set_focus(id);
        press_key(&mut gui, NamedKey::Space);
        laid_out(&mut gui);

        for _ in 0..6 {
            press_key(&mut gui, NamedKey::ArrowDown);
        }

        let field = drop_down(&gui, id);

        assert_eq!(field.highlighted(), 6);
        assert_eq!(field.first_visible(), 3, "6 が下端に来るまで送る");
        assert!(field.item_rect(6).is_some());
    }

    #[test]
    fn opening_reveals_the_current_choice() {
        let many: Vec<String> = (0..20).map(|index| format!("row{index}")).collect();
        let (mut gui, id) = with_dropdown(Dropdown::new(many).max_visible(4).selected(15));

        laid_out(&mut gui);
        press_center(&mut gui, id);
        laid_out(&mut gui);

        let field = drop_down(&gui, id);

        assert_eq!(field.highlighted(), 15);
        assert!(field.item_rect(15).is_some(), "選んでいる行が見えている");
    }

    // --- 見た目 ---

    #[test]
    fn the_closed_label_shows_the_choice_or_the_placeholder() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).placeholder("選ぶ"));
        laid_out(&mut gui);

        let label = |gui: &mut Gui| {
            let list = gui.paint();

            list.commands_of(id)
                .find_map(|command| match &command.primitive {
                    Primitive::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
        };

        assert_eq!(label(&mut gui).as_deref(), Some("選ぶ"));

        gui.tree_mut().get_as_mut::<Dropdown>(id).unwrap().select(3);
        gui.tree_mut().request_layout(id);
        laid_out(&mut gui);

        assert_eq!(label(&mut gui).as_deref(), Some("item3"));
    }

    #[test]
    fn the_highlighted_and_chosen_rows_get_different_colours() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).selected(0));
        laid_out(&mut gui);
        gui.set_focus(id);

        press_key(&mut gui, NamedKey::Space);
        laid_out(&mut gui);
        // 指しているのは 2 行目、選ばれているのは 1 行目。
        press_key(&mut gui, NamedKey::ArrowDown);

        let highlighted = drop_down(&gui, id).item_rect(1).unwrap();
        let chosen = drop_down(&gui, id).item_rect(0).unwrap();

        let colour_at = |gui: &mut Gui, rect: Rect| {
            let list = gui.paint();

            list.commands_of(id)
                .filter_map(|command| match command.primitive {
                    Primitive::Rect { rect: at, color, .. } if at.size() == rect.size() => {
                        Some((at, color))
                    }
                    _ => None,
                })
                .find(|(at, _)| (at.y - rect.y).abs() < 0.01)
                .map(|(_, color)| color)
        };

        // 浮かせたぶんは画面座標なので、局所座標にずらして照合する。
        let origin = gui.tree().bounds(id).origin();
        let highlighted = highlighted.translate(origin);
        let chosen = chosen.translate(origin);

        assert_eq!(colour_at(&mut gui, highlighted), Some(Color::hex(0x3355aa)));
        assert_eq!(colour_at(&mut gui, chosen), Some(Color::hex(0x114422)));
    }

    #[test]
    fn the_popup_rows_are_clipped_to_the_shell() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        press_center(&mut gui, id);
        laid_out(&mut gui);

        let origin = gui.tree().bounds(id).origin();
        let popup = drop_down(&gui, id).popup_rect().translate(origin);

        let list = gui.paint();

        // 下敷きは切り抜かない。行は下敷きで切り抜く。
        let clipped: Vec<_> = list
            .commands_of(id)
            .filter_map(|command| command.clip)
            .collect();

        assert!(!clipped.is_empty(), "行は切り抜かれる");

        for clip in clipped {
            assert_eq!(clip.rect, popup);
        }
    }

    // --- その他 ---

    #[test]
    fn it_is_wide_enough_for_the_longest_item() {
        let (mut gui, narrow) =
            with_dropdown(Dropdown::new(["a"]).min_width(0.0));
        laid_out(&mut gui);
        let narrow_width = gui.tree().bounds(narrow).width;

        let (mut gui, wide) =
            with_dropdown(Dropdown::new(["a", "aaaaaaaaaaaaaaaaaaaa"]).min_width(0.0));
        laid_out(&mut gui);

        assert!(
            gui.tree().bounds(wide).width > narrow_width,
            "長い項目に合わせる"
        );
    }

    #[test]
    fn replacing_the_items_drops_the_choice() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()).selected(3));
        laid_out(&mut gui);

        press_center(&mut gui, id);
        assert!(drop_down(&gui, id).is_open());

        gui.tree_mut()
            .get_as_mut::<Dropdown>(id)
            .unwrap()
            .set_items(["x", "y"]);
        gui.tree_mut().request_layout(id);
        laid_out(&mut gui);

        let field = drop_down(&gui, id);

        assert_eq!(field.selected_index(), None);
        assert!(!field.is_open(), "入れ替えたら閉じる");
        assert_eq!(field.items().len(), 2);
    }

    #[test]
    fn a_disabled_dropdown_takes_nothing() {
        let (mut gui, id) = with_dropdown(Dropdown::new(items()));
        laid_out(&mut gui);

        gui.tree_mut().set_disabled(id, true);
        press_center(&mut gui, id);

        assert!(!drop_down(&gui, id).is_open());
        let centre = gui.tree().bounds(id).center();
        assert_eq!(gui.hit_test(centre), WidgetId::NONE);
    }

    #[test]
    fn two_dropdowns_do_not_both_open() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let first = gui
            .tree_mut()
            .add_child(root, Box::new(Dropdown::new(items())));
        let second = gui
            .tree_mut()
            .add_child(root, Box::new(Dropdown::new(items())));

        laid_out(&mut gui);

        press_center(&mut gui, first);
        laid_out(&mut gui);
        assert!(drop_down(&gui, first).is_open());

        // 2 つめを押す。1 つめは焦点を失って閉じる。
        let at = gui.tree().bounds(second).center();

        // 1 つめの一覧が 2 つめに重なっているので、まず外して閉じさせる。
        press_at(&mut gui, Point::new(VIEWPORT.width - 2.0, VIEWPORT.height - 2.0));
        laid_out(&mut gui);
        assert!(!drop_down(&gui, first).is_open());

        press_at(&mut gui, at);
        laid_out(&mut gui);

        assert!(drop_down(&gui, second).is_open());
        assert!(!drop_down(&gui, first).is_open());
    }
}
