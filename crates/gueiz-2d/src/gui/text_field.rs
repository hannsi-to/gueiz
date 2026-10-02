//! 1 行の入力欄。
//!
//! # 押した場所をバイト位置に直す
//!
//! 入力欄でいちばん面倒なのはここです。「x = 73 px を押した」から
//! 「文字列の 5 バイト目」を出すには、**1 文字ずつの送り幅**が要ります。
//!
//! ところが[測る道具](crate::gui::context::TextMeasure)を借りられるのは
//! 測る周回のあいだだけで、押されたときには手元にありません。
//! 入力は [`Gui::handle_input`](crate::gui::Gui::handle_input) で来るので、
//! そこに書体を持ち込むと**入力を流すたびに書体を渡すことになります。**
//!
//! そこで [`TextField`] は、測る周回で**文字の境目の x を全部覚えます**。
//!
//! ```text
//! "Hello"
//!  │ │ │ │ │ │
//!  0 8 16 20 24 32   ← caret_offsets: [(0,0), (1,8), (2,16), (3,20), (4,24), (5,32)]
//! ```
//!
//! 押されたときはこの表を引くだけです。表は**字が変わったときにだけ**
//! 作り直します（字を変えたら [`EventContext::request_layout`] を呼ぶので、
//! 次の測る周回で作り直されます）。
//!
//! 作るのに前方一致で何度も測るので字数の 2 乗かかりますが、1 行ぶんなので
//! 問題になりません。1 文字ずつ測って足すと詰め（カーニング）のぶんずれます。
//!
//! # 選んでいる範囲
//!
//! [`TextField::anchor`] と [`TextField::caret`] の 2 つで表します。
//! 同じなら選んでいません。**どちらが前かは決めません** ―
//! 右から左へ選ぶと `caret < anchor` になります。
//!
//! # 横に送る
//!
//! 字が枠より長くなったら、**キャレットが見えるところまで横に送ります**。
//! 送り量は [`TextField::scroll`]。これも押した場所の計算に効くので、
//! 表を引く前に足し引きします。
//!
//! # やっていないこと
//!
//! - **複数行。** 行の折り返しとキャレットの上下移動が入ると別物になります
//! - **切り貼り。** 紙挟み（クリップボード）は窓の層のものなので、
//!   `gueiz-2d` からは触れません。`Cmd+C` を受け取って
//!   [`TextField::selected_text`] を呼ぶのは呼ぶ側の仕事です
//! - **語単位の移動。** `Alt+←` などは
//!   [`TextField::move_caret`] を外から呼べば足せます
//! - **かな漢字変換の途中の表示。** 確定した字が [`Event::Text`] で来るだけです
//!
//! # 使い方
//!
//! ```no_run
//! # use gueiz_2d::gui::id::Tag;
//! # use gueiz_2d::gui::text_field::TextField;
//! # use gueiz_2d::gui::widget::ActionKind;
//! # use gueiz_2d::gui::Gui;
//! # fn build(gui: &mut Gui, parent: gueiz_2d::gui::id::WidgetId) {
//! const NAME: Tag = Tag::new("name");
//!
//! let field = gui.tree_mut().add_child(
//!     parent,
//!     Box::new(TextField::new().placeholder("名前")),
//! );
//! gui.tree_mut().set_tag(field, NAME);
//!
//! for action in gui.drain_actions() {
//!     if !action.is(NAME) {
//!         continue;
//!     }
//!
//!     match action.kind {
//!         // 字が変わった。
//!         ActionKind::ValueChanged => {}
//!         // Enter が押された。
//!         ActionKind::Submitted => {}
//!         // Escape が押された。
//!         ActionKind::Cancelled => {}
//!         _ => {}
//!     }
//! }
//! # }
//! ```
//!
//! [`Event::Text`]: crate::gui::event::Event::Text
//! [`EventContext::request_layout`]: crate::gui::EventContext::request_layout

use crate::gui::event::{Event, EventResult, Key, NamedKey, PointerButton};
use crate::gui::geometry::{Align, Rect, Size};
use crate::gui::layout::Constraints;
use crate::gui::painter::{Painter, TextLayoutOptions};
use crate::gui::theme::Role;
use crate::gui::widget::{ActionKind, Behavior};
use crate::gui::{ArrangeContext, EventContext, MeasureContext, PaintContext, Widget};

/// キャレットの太さ。
const CARET_WIDTH: f32 = 1.5;

/// 字が無いときでも、この幅は確保する。
const DEFAULT_MIN_WIDTH: f32 = 120.0;

/// 1 行の入力欄。
pub struct TextField {
    text: String,
    /// 選び始めた位置。バイト位置。
    anchor: usize,
    /// いまのキャレット。バイト位置。
    caret: usize,

    placeholder: Option<String>,
    role: Role,
    /// 選んだ範囲とキャレットの色を引く役。
    selection_role: Role,
    /// 何も入っていないときに出す字の役。
    placeholder_role: Role,

    min_width: f32,
    /// 入れられるバイト数の上限。`None` なら無制限。
    max_bytes: Option<usize>,
    /// 字を隠す。入れると 1 文字につき `mask` を 1 つ出す。
    mask: Option<char>,

    // --- 測る周回が埋める ---
    /// 文字の境目の x。`(バイト位置, 左端からの x)`。**昇順。**
    caret_offsets: Vec<(usize, f32)>,
    /// 字が使う幅。
    text_width: f32,
    /// 字を置ける範囲（枠から余白を引いたもの）。局所座標。
    inner: Rect,

    // --- 触られている状態 ---
    /// 押して引いているあいだ立つ。
    dragging: bool,
    /// 横に送った量。0 以上。
    scroll: f32,
}

impl Default for TextField {
    fn default() -> Self {
        Self::new()
    }
}

impl TextField {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            anchor: 0,
            caret: 0,
            placeholder: None,
            role: Role::CONTROL,
            selection_role: Role::ACCENT,
            placeholder_role: Role::TEXT,
            min_width: DEFAULT_MIN_WIDTH,
            max_bytes: None,
            mask: None,
            caret_offsets: Vec::new(),
            text_width: 0.0,
            inner: Rect::ZERO,
            dragging: false,
            scroll: 0.0,
        }
    }

    /// 最初から字を入れておく。
    pub fn with_text(text: impl Into<String>) -> Self {
        let mut field = Self::new();
        field.text = text.into();
        field.caret = field.text.len();
        field.anchor = field.caret;

        field
    }

    /// 何も入っていないときに出す字。
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// 選んだ範囲（`background`）とキャレット（`foreground`）の色を引く役。
    pub fn selection_role(mut self, selection_role: Role) -> Self {
        self.selection_role = selection_role;
        self
    }

    pub fn placeholder_role(mut self, placeholder_role: Role) -> Self {
        self.placeholder_role = placeholder_role;
        self
    }

    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width.max(0.0);
        self
    }

    /// 入れられるバイト数の上限。
    ///
    /// **バイト**で数えます。日本語は 1 文字 3 バイトなので、
    /// 文字数で縛りたいなら [`TextField::text`] を見て呼ぶ側で弾いてください。
    /// 境目で切らないので、上限で文字が壊れることはありません。
    pub fn max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    /// 字を隠す。合言葉の入力に。
    ///
    /// 置き換えるのは**見た目だけ**で、[`TextField::text`] は素の字を返します。
    pub fn mask(mut self, mask: char) -> Self {
        self.mask = Some(mask);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// 字を入れ替える。キャレットは末尾へ。
    ///
    /// **呼んだら
    /// [`WidgetTree::request_layout`](crate::gui::tree::WidgetTree::request_layout)
    /// も呼んでください。**境目の表を作り直す必要があります。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.scroll = 0.0;
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// 横に送っている量。
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// 選んでいる範囲。前から後ろの順。選んでいなければ空（`start == end`）。
    pub fn selection(&self) -> (usize, usize) {
        (self.anchor.min(self.caret), self.anchor.max(self.caret))
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    /// 選んでいる字。
    pub fn selected_text(&self) -> &str {
        let (start, end) = self.selection();

        &self.text[start..end]
    }

    /// 全部選ぶ。
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// 選ぶのをやめる。キャレットはそのまま。
    pub fn clear_selection(&mut self) {
        self.anchor = self.caret;
    }

    /// キャレットを動かす。`extend` を立てると選んだ範囲を伸ばす。
    ///
    /// 境目に合わない位置を渡しても、**手前の境目に丸めます。**
    pub fn move_caret(&mut self, to: usize, extend: bool) {
        self.caret = self.floor_boundary(to);

        if !extend {
            self.anchor = self.caret;
        }
    }

    /// 選んでいる字を差し替える。選んでいなければキャレットに差し込む。
    ///
    /// 入った字のぶんキャレットが進みます。上限を超えるぶんは入りません。
    pub fn replace_selection(&mut self, with: &str) -> bool {
        let (start, end) = self.selection();

        // 入れ替えたあとの長さが上限を超えないか。
        if let Some(max) = self.max_bytes {
            let after = self.text.len() - (end - start) + with.len();

            if after > max {
                return false;
            }
        }

        if start == end && with.is_empty() {
            return false;
        }

        self.text.replace_range(start..end, with);
        self.caret = start + with.len();
        self.anchor = self.caret;

        true
    }

    /// 手前の 1 文字を消す。選んでいればそれを消す。
    pub fn delete_backward(&mut self) -> bool {
        if self.has_selection() {
            return self.replace_selection("");
        }

        if self.caret == 0 {
            return false;
        }

        let start = self.previous_boundary(self.caret);
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.anchor = start;

        true
    }

    /// 次の 1 文字を消す。選んでいればそれを消す。
    pub fn delete_forward(&mut self) -> bool {
        if self.has_selection() {
            return self.replace_selection("");
        }

        if self.caret >= self.text.len() {
            return false;
        }

        let end = self.next_boundary(self.caret);
        self.text.replace_range(self.caret..end, "");
        self.anchor = self.caret;

        true
    }

    /// その x（局所座標）に近い境目。送った量も見る。
    ///
    /// 表がまだ無ければ 0。
    pub fn boundary_at(&self, x: f32) -> usize {
        // 枠の左端からではなく、字の左端からの距離に直す。
        let target = x - self.inner.x + self.scroll;

        let mut best = 0;
        let mut best_distance = f32::INFINITY;

        for (byte, offset) in &self.caret_offsets {
            let distance = (offset - target).abs();

            // 同じ近さなら奥を取る。字のちょうど真ん中を押したとき、
            // キャレットがその字の**後ろ**に来る（書き足すほうが多いので）。
            if distance <= best_distance {
                best_distance = distance;
                best = *byte;
            }
        }

        best
    }

    /// その境目の x。字の左端からの距離。
    pub fn offset_of(&self, byte: usize) -> f32 {
        self.caret_offsets
            .iter()
            .find(|(at, _)| *at == byte)
            .map(|(_, offset)| *offset)
            // 表に無い。末尾として扱う。
            .unwrap_or(self.text_width)
    }

    /// 見た目に出す字。隠す設定なら置き換えたもの。
    fn displayed(&self) -> String {
        match self.mask {
            Some(mask) => self.text.chars().map(|_| mask).collect(),
            None => self.text.clone(),
        }
    }

    fn previous_boundary(&self, from: usize) -> usize {
        self.text[..from]
            .char_indices()
            .next_back()
            .map(|(at, _)| at)
            .unwrap_or(0)
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.text[from..]
            .char_indices()
            .nth(1)
            .map(|(at, _)| from + at)
            .unwrap_or(self.text.len())
    }

    /// 境目に丸める。
    fn floor_boundary(&self, at: usize) -> usize {
        let at = at.min(self.text.len());

        if self.text.is_char_boundary(at) {
            return at;
        }

        self.text[..at]
            .char_indices()
            .next_back()
            .map(|(boundary, _)| boundary)
            .unwrap_or(0)
    }

    /// キャレットが見えるところまで横に送る。
    fn reveal_caret(&mut self) {
        let caret = self.offset_of(self.caret);
        let width = self.inner.width;

        if width <= 0.0 {
            self.scroll = 0.0;
            return;
        }

        // 左にはみ出した。
        if caret < self.scroll {
            self.scroll = caret;
        }

        // 右にはみ出した。キャレットの線のぶんも見えるようにする。
        if caret - self.scroll > width - CARET_WIDTH {
            self.scroll = caret - width + CARET_WIDTH;
        }

        // 字が枠より短いなら送らない。送ると右に空白が出る。
        //
        // 上限に `CARET_WIDTH` を足しているのは、**末尾のキャレットのため**。
        // 字の右端ぴったりで止めると、末尾に立てた線が枠の外に出て見えない。
        let max = (self.text_width + CARET_WIDTH - width).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }

    fn text_options(&self, text_size: f32) -> TextLayoutOptions {
        TextLayoutOptions {
            size: text_size,
            // 送りは自分で面倒を見るので、寄せは常に左上。
            horizontal: Align::Start,
            vertical: Align::Center,
            wrap: false,
        }
    }

    /// 境目の表を作り直す。測る周回からだけ呼ぶ。
    fn rebuild_offsets(&mut self, context: &MeasureContext<'_>, text_size: f32) {
        let displayed = self.displayed();
        let options = self.text_options(text_size);

        self.caret_offsets.clear();
        self.caret_offsets.push((0, 0.0));

        // 前方一致で測る。1 文字ずつ測って足すと、詰めのぶんずれる。
        for (byte, _) in displayed.char_indices().skip(1) {
            let width = context
                .measure_text(&displayed[..byte], options, None)
                .width;

            self.caret_offsets.push((byte, width));
        }

        let total = context.measure_text(&displayed, options, None).width;
        self.caret_offsets.push((displayed.len(), total));
        self.text_width = total;

        // 隠す設定だと、表のバイト位置が素の字と食い違う。素のほうへ直す。
        if self.mask.is_some() {
            let mut sources = self.text.char_indices().map(|(at, _)| at);

            for (byte, _) in self.caret_offsets.iter_mut() {
                *byte = sources.next().unwrap_or(self.text.len());
            }
        }
    }

    /// 選んだ範囲を表す矩形。局所座標。選んでいなければ `None`。
    pub fn selection_rect(&self) -> Option<Rect> {
        if !self.has_selection() {
            return None;
        }

        let (start, end) = self.selection();
        let left = self.offset_of(start) - self.scroll;
        let right = self.offset_of(end) - self.scroll;

        Some(Rect::new(
            self.inner.x + left,
            self.inner.y,
            right - left,
            self.inner.height,
        ))
    }

    /// キャレットの線。局所座標。
    pub fn caret_rect(&self) -> Rect {
        let x = self.inner.x + self.offset_of(self.caret) - self.scroll;

        Rect::new(x, self.inner.y, CARET_WIDTH, self.inner.height)
    }
}

impl Widget for TextField {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let style = context.theme().style(self.role, context.state());
        let text_size = style.text_size;

        self.rebuild_offsets(context, text_size);

        let line = context.line_height(text_size);
        let height = (line + style.padding.vertical()).max(context.metrics().control_height);
        let width = self.min_width + style.padding.horizontal();

        constraints.constrain(Size::new(width, height))
    }

    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        let style = context.theme().style(self.role, context.state());

        self.inner = bounds.deflate(style.padding);

        // 枠が広がって、送りすぎになっていることがある。
        self.reveal_caret();
    }

    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let state = context.state();
        let style = context.theme().style(self.role, state);
        let frame = context.rect();

        painter.rounded_rect(frame, style.corners, style.background);

        if style.border_width > 0.0 {
            painter.border(frame, style.corners, style.border_width, style.border);
        }

        // 字は枠からはみ出す。自分で切り抜く。
        let inner = self.inner;

        painter.with_clip(inner, style.corners, |painter| {
            let selection = context.theme().style(self.selection_role, state);

            // 選んだ範囲は字の下。
            if let Some(rect) = self.selection_rect() {
                painter.rect(rect, selection.background);
            }

            let text_options = self.text_options(style.text_size);
            let left = inner.x - self.scroll;

            if self.text.is_empty() {
                // 何も入っていない。案内の字を出す。キャレットは出す。
                if let Some(placeholder) = &self.placeholder {
                    let hint = context.theme().style(self.placeholder_role, state);

                    painter.text(
                        Rect::new(inner.x, inner.y, inner.width, inner.height),
                        placeholder.as_str(),
                        text_options,
                        hint.foreground,
                    );
                }
            } else {
                painter.text(
                    // 幅は字のぶん取る。足りないと折り返しも寄せも狂う。
                    Rect::new(left, inner.y, self.text_width.max(inner.width), inner.height),
                    self.displayed(),
                    text_options,
                    style.foreground,
                );
            }

            // キャレットは焦点があるときだけ。選んでいるあいだも出す
            // （どちらへ伸びているかが分かる）。
            if state.focused && !state.disabled {
                painter.rect(self.caret_rect(), selection.foreground);
            }
        });
    }

    fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
        match event {
            Event::Text(text) => {
                // 改行は 1 行の欄には入れない。Enter は確定として扱う。
                let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();

                if cleaned.is_empty() {
                    return EventResult::Consumed;
                }

                if self.replace_selection(&cleaned) {
                    self.after_edit(context);
                }

                EventResult::Consumed
            }

            Event::KeyPressed { key, .. } => self.on_key(context, key),

            Event::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                let at = self.boundary_at(position.x);

                // Shift を押しながらなら、いまの始点から伸ばす。
                self.move_caret(at, context.modifiers().shift());

                self.dragging = true;
                context.capture_pointer();
                self.reveal_caret();

                EventResult::Consumed
            }

            Event::PointerMove { position, .. } if self.dragging => {
                let at = self.boundary_at(position.x);
                self.move_caret(at, true);
                self.reveal_caret();

                // 枠の外まで引いたら、送って追いかける。
                context.request_layout();

                EventResult::Consumed
            }

            Event::PointerReleased { .. } if self.dragging => {
                self.dragging = false;
                context.release_pointer();

                EventResult::Consumed
            }

            // 焦点が来たら全部選ぶ、のような作りにはしない。
            // 押した場所にキャレットが来るほうが、書き足すときに楽。
            Event::FocusLost => {
                self.dragging = false;
                self.clear_selection();

                EventResult::Ignored
            }

            // 押されたことは受け取る。後ろのパネルに抜けないように。
            event if event.is_pointer() => EventResult::Consumed,

            _ => EventResult::Ignored,
        }
    }

    fn behavior(&self) -> Behavior {
        Behavior::CONTROL
    }
}

impl TextField {
    /// 鍵が押された。
    fn on_key(&mut self, context: &mut EventContext<'_>, key: &Key) -> EventResult {
        let shift = context.modifiers().shift();
        let command = context.modifiers().command();

        // 全部選ぶ。
        if command && key.is_character('a') {
            self.select_all();
            return EventResult::Consumed;
        }

        // 切り貼りは紙挟みが要るので、ここでは受け取らない。
        // 外から `selected_text` と `replace_selection` を呼んでください。
        if command {
            return EventResult::Ignored;
        }

        let Key::Named(named) = key else {
            // 文字の鍵は `Event::Text` で来る。こちらでは何もしない。
            return EventResult::Ignored;
        };

        match named {
            NamedKey::Backspace => {
                if self.delete_backward() {
                    self.after_edit(context);
                }

                EventResult::Consumed
            }

            NamedKey::Delete => {
                if self.delete_forward() {
                    self.after_edit(context);
                }

                EventResult::Consumed
            }

            NamedKey::ArrowLeft => {
                // 選んでいて伸ばさないなら、選んだ手前へ畳む。
                let to = if self.has_selection() && !shift {
                    self.selection().0
                } else {
                    self.previous_boundary(self.caret)
                };

                self.move_caret(to, shift);
                self.reveal_caret();
                context.request_layout();

                EventResult::Consumed
            }

            NamedKey::ArrowRight => {
                let to = if self.has_selection() && !shift {
                    self.selection().1
                } else {
                    self.next_boundary(self.caret)
                };

                self.move_caret(to, shift);
                self.reveal_caret();
                context.request_layout();

                EventResult::Consumed
            }

            NamedKey::Home => {
                self.move_caret(0, shift);
                self.reveal_caret();
                context.request_layout();

                EventResult::Consumed
            }

            NamedKey::End => {
                self.move_caret(self.text.len(), shift);
                self.reveal_caret();
                context.request_layout();

                EventResult::Consumed
            }

            NamedKey::Enter => {
                context.emit(ActionKind::Submitted);
                EventResult::Consumed
            }

            NamedKey::Escape => {
                context.emit(ActionKind::Cancelled);
                EventResult::Consumed
            }

            // 上下は 1 行では行き先が無い。Tab は焦点の移動に使う。
            _ => EventResult::Ignored,
        }
    }

    /// 字が変わったあとの始末。
    fn after_edit(&mut self, context: &mut EventContext<'_>) {
        // 境目の表を作り直してもらう。これを呼ばないと、次に押した場所が
        // 古い表で引かれてずれる。
        context.request_layout();
        context.emit(ActionKind::ValueChanged);

        // 表はまだ古いので、ここで送り直しても正しくない。
        // 次の `arrange` が `reveal_caret` を呼ぶのでそちらに任せる。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Color;
    use crate::gui::container::Stack;
    use crate::gui::context::TextMeasure;
    use crate::gui::event::{InputEvent, Modifiers};
    use crate::gui::geometry::{Insets, Point};
    use crate::gui::id::WidgetId;
    use crate::gui::painter::Primitive;
    use crate::gui::theme::{Metrics, StateKey, Style, Theme};
    use crate::gui::Gui;

    /// 1 文字 `size * 0.5` 幅。詰めは無し。
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
        width: 300.0,
        height: 100.0,
    };

    /// 字 1 つぶんの幅。文字の大きさ 16 なので 8。
    const ADVANCE: f32 = 8.0;
    /// 余白。局所座標の x に足すと字の左端。
    const PAD: f32 = 4.0;

    fn theme() -> Theme {
        let mut theme = Theme::new();

        theme.set_metrics(Metrics {
            spacing: 4.0,
            text_size: 16.0,
            control_height: 28.0,
            line_height_factor: 1.0,
        });

        theme.set(
            Role::CONTROL,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x101014))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::all(PAD)),
        );
        theme.set(
            Role::ACCENT,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x335577))
                .foreground(Color::hex(0xffcc00)),
        );
        theme.set(
            Role::TEXT,
            StateKey::Normal,
            Style::BARE.foreground(Color::hex(0x666666)),
        );

        theme
    }

    fn with_field(field: TextField) -> (Gui, WidgetId) {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let id = gui.tree_mut().add_child(root, Box::new(field));

        (gui, id)
    }

    fn laid_out(gui: &mut Gui) {
        gui.layout(VIEWPORT, &Monospace);
    }

    fn field(gui: &Gui, id: WidgetId) -> &TextField {
        gui.tree().get_as::<TextField>(id).expect("TextField")
    }

    /// 字を打つ。
    fn type_text(gui: &mut Gui, text: &str) {
        gui.handle_input(InputEvent::Text(String::from(text)));
    }

    fn press(gui: &mut Gui, named: NamedKey) {
        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(named),
            repeat: false,
        });
    }

    fn press_with(gui: &mut Gui, named: NamedKey, modifiers: Modifiers) {
        gui.handle_input(InputEvent::ModifiersChanged(modifiers));
        press(gui, named);
        gui.handle_input(InputEvent::ModifiersChanged(Modifiers::empty()));
    }

    // --- 打つ・消す ---

    #[test]
    fn typing_inserts_at_the_caret() {
        let (mut gui, id) = with_field(TextField::new());
        laid_out(&mut gui);
        gui.set_focus(id);

        type_text(&mut gui, "ab");
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "ab");
        assert_eq!(field(&gui, id).caret(), 2);

        // 手前へ戻して差し込む。
        press(&mut gui, NamedKey::ArrowLeft);
        type_text(&mut gui, "X");
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "aXb");
        assert_eq!(field(&gui, id).caret(), 2);
    }

    #[test]
    fn control_characters_are_filtered_out() {
        let (mut gui, id) = with_field(TextField::new());
        laid_out(&mut gui);
        gui.set_focus(id);

        // 改行やタブは 1 行の欄に入れない。
        type_text(&mut gui, "a\nb\tc");
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "abc");
    }

    #[test]
    fn backspace_and_delete_work_on_characters() {
        let (mut gui, id) = with_field(TextField::with_text("abc"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press(&mut gui, NamedKey::Backspace);
        laid_out(&mut gui);
        assert_eq!(field(&gui, id).text(), "ab");

        press(&mut gui, NamedKey::Home);
        press(&mut gui, NamedKey::Delete);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "b");
        assert_eq!(field(&gui, id).caret(), 0);

        // 端では何も起きない。
        press(&mut gui, NamedKey::Backspace);
        press(&mut gui, NamedKey::End);
        press(&mut gui, NamedKey::Delete);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "b");
    }

    #[test]
    fn multibyte_characters_move_one_at_a_time() {
        let (mut gui, id) = with_field(TextField::with_text("あいう"));
        laid_out(&mut gui);
        gui.set_focus(id);

        assert_eq!(field(&gui, id).caret(), 9, "3 文字 × 3 バイト");

        press(&mut gui, NamedKey::ArrowLeft);
        assert_eq!(field(&gui, id).caret(), 6, "1 文字ぶん戻る");

        press(&mut gui, NamedKey::Backspace);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "あう", "バイトではなく文字で消える");
    }

    #[test]
    fn the_byte_limit_rejects_the_overflow() {
        let (mut gui, id) = with_field(TextField::new().max_bytes(4));
        laid_out(&mut gui);
        gui.set_focus(id);

        type_text(&mut gui, "abcd");
        laid_out(&mut gui);
        assert_eq!(field(&gui, id).text(), "abcd");

        // 入らない。**途中まで入れて切る、にはしない。**
        type_text(&mut gui, "e");
        laid_out(&mut gui);
        assert_eq!(field(&gui, id).text(), "abcd");

        // 日本語は 1 文字 3 バイト。
        let (mut gui, id) = with_field(TextField::new().max_bytes(4));
        laid_out(&mut gui);
        gui.set_focus(id);

        type_text(&mut gui, "あ");
        type_text(&mut gui, "い");
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "あ", "2 文字目は 6 バイトで溢れる");
    }

    // --- 選ぶ ---

    #[test]
    fn shift_arrow_extends_the_selection() {
        let (mut gui, id) = with_field(TextField::with_text("hello"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press(&mut gui, NamedKey::Home);
        assert!(!field(&gui, id).has_selection());

        press_with(&mut gui, NamedKey::ArrowRight, Modifiers::SHIFT);
        press_with(&mut gui, NamedKey::ArrowRight, Modifiers::SHIFT);

        assert_eq!(field(&gui, id).selection(), (0, 2));
        assert_eq!(field(&gui, id).selected_text(), "he");

        // 伸ばさずに動かすと、選ぶのをやめる。
        press(&mut gui, NamedKey::ArrowRight);
        assert!(!field(&gui, id).has_selection());
    }

    #[test]
    fn selecting_right_to_left_keeps_the_anchor_behind() {
        let (mut gui, id) = with_field(TextField::with_text("hello"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press_with(&mut gui, NamedKey::ArrowLeft, Modifiers::SHIFT);
        press_with(&mut gui, NamedKey::ArrowLeft, Modifiers::SHIFT);

        let text_field = field(&gui, id);

        assert_eq!(text_field.anchor(), 5);
        assert_eq!(text_field.caret(), 3);
        assert_eq!(text_field.selection(), (3, 5), "前後に並べ直して返す");
        assert_eq!(text_field.selected_text(), "lo");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let (mut gui, id) = with_field(TextField::with_text("hello"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.tree_mut().get_as_mut::<TextField>(id).unwrap().select_all();
        type_text(&mut gui, "bye");
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "bye");
        assert_eq!(field(&gui, id).caret(), 3);
        assert!(!field(&gui, id).has_selection());
    }

    #[test]
    fn backspace_deletes_the_selection() {
        let (mut gui, id) = with_field(TextField::with_text("hello"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press(&mut gui, NamedKey::Home);
        press_with(&mut gui, NamedKey::ArrowRight, Modifiers::SHIFT);
        press_with(&mut gui, NamedKey::ArrowRight, Modifiers::SHIFT);
        press(&mut gui, NamedKey::Backspace);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).text(), "llo");
    }

    #[test]
    fn collapsing_a_selection_goes_to_the_near_end() {
        let (mut gui, id) = with_field(TextField::with_text("hello"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.tree_mut().get_as_mut::<TextField>(id).unwrap().select_all();

        press(&mut gui, NamedKey::ArrowLeft);
        assert_eq!(field(&gui, id).caret(), 0, "左なら手前へ畳む");

        gui.tree_mut().get_as_mut::<TextField>(id).unwrap().select_all();

        press(&mut gui, NamedKey::ArrowRight);
        assert_eq!(field(&gui, id).caret(), 5, "右なら奥へ畳む");
    }

    #[test]
    fn command_a_selects_everything() {
        let (mut gui, id) = with_field(TextField::with_text("hello"));
        laid_out(&mut gui);
        gui.set_focus(id);

        let command = if cfg!(target_os = "macos") {
            Modifiers::SUPER
        } else {
            Modifiers::CONTROL
        };

        gui.handle_input(InputEvent::ModifiersChanged(command));
        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Character('a'),
            repeat: false,
        });

        assert_eq!(field(&gui, id).selected_text(), "hello");
    }

    // --- 押した場所 ---

    #[test]
    fn clicking_puts_the_caret_at_the_nearest_boundary() {
        let (mut gui, id) = with_field(TextField::with_text("abcdef"));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);

        // 字の左端は余白のぶん右。3 文字目の真ん中を押す。
        let at = Point::new(bounds.x + PAD + ADVANCE * 2.5, bounds.center().y);
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });

        // ちょうど真ん中なので、奥の境目（3）に着く。
        assert_eq!(field(&gui, id).caret(), 3);
        assert_eq!(gui.focused(), id, "押したら焦点も来る");
    }

    #[test]
    fn clicking_past_the_end_goes_to_the_end() {
        let (mut gui, id) = with_field(TextField::with_text("ab"));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(bounds.right() - 1.0, bounds.center().y),
            button: PointerButton::Primary,
        });

        assert_eq!(field(&gui, id).caret(), 2);
    }

    #[test]
    fn clicking_on_multibyte_text_lands_on_character_boundaries() {
        let (mut gui, id) = with_field(TextField::with_text("あいう"));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);

        // 2 文字目の真ん中。
        let at = Point::new(bounds.x + PAD + ADVANCE * 1.5, bounds.center().y);
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });

        let caret = field(&gui, id).caret();

        assert!(
            caret == 3 || caret == 6,
            "文字の境目に乗る: {caret}"
        );
        assert!(
            field(&gui, id).text().is_char_boundary(caret),
            "バイトの途中を指さない"
        );
    }

    #[test]
    fn dragging_selects() {
        let (mut gui, id) = with_field(TextField::with_text("abcdef"));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        let y = bounds.center().y;
        let start = Point::new(bounds.x + PAD, y);

        gui.handle_input(InputEvent::PointerPressed {
            position: start,
            button: PointerButton::Primary,
        });
        assert_eq!(gui.pointer_capture(), id);

        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(bounds.x + PAD + ADVANCE * 3.0, y),
        });
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).selected_text(), "abc");

        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(bounds.x + PAD + ADVANCE * 3.0, y),
            button: PointerButton::Primary,
        });

        assert_eq!(gui.pointer_capture(), WidgetId::NONE);
        assert_eq!(field(&gui, id).selected_text(), "abc", "放しても残る");
    }

    #[test]
    fn shift_clicking_extends_from_the_caret() {
        let (mut gui, id) = with_field(TextField::with_text("abcdef"));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);
        let y = bounds.center().y;

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(bounds.x + PAD, y),
            button: PointerButton::Primary,
        });
        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(bounds.x + PAD, y),
            button: PointerButton::Primary,
        });

        gui.handle_input(InputEvent::ModifiersChanged(Modifiers::SHIFT));
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(bounds.x + PAD + ADVANCE * 4.0, y),
            button: PointerButton::Primary,
        });

        assert_eq!(field(&gui, id).selected_text(), "abcd");
    }

    // --- 伝達 ---

    #[test]
    fn editing_reports_a_value_change() {
        let (mut gui, id) = with_field(TextField::new());
        laid_out(&mut gui);
        gui.set_focus(id);

        type_text(&mut gui, "a");

        let actions = gui.drain_actions();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(actions[0].kind, ActionKind::ValueChanged);
        assert_eq!(actions[0].widget, id);

        // 動かしただけでは出ない。
        laid_out(&mut gui);
        press(&mut gui, NamedKey::ArrowLeft);
        assert!(gui.drain_actions().is_empty());
    }

    #[test]
    fn enter_submits_and_escape_cancels() {
        let (mut gui, id) = with_field(TextField::with_text("x"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press(&mut gui, NamedKey::Enter);
        let actions = gui.drain_actions();
        assert_eq!(actions[0].kind, ActionKind::Submitted);

        press(&mut gui, NamedKey::Escape);
        let actions = gui.drain_actions();
        assert_eq!(actions[0].kind, ActionKind::Cancelled);

        // どちらも字は変えない。
        assert_eq!(field(&gui, id).text(), "x");
    }

    #[test]
    fn a_rejected_edit_reports_nothing() {
        let (mut gui, id) = with_field(TextField::new().max_bytes(1));
        laid_out(&mut gui);
        gui.set_focus(id);

        type_text(&mut gui, "a");
        assert_eq!(gui.drain_actions().len(), 1);

        laid_out(&mut gui);

        // 入らなかった。何も伝えない。
        type_text(&mut gui, "b");
        assert!(gui.drain_actions().is_empty());
    }

    // --- 横送り ---

    #[test]
    fn the_caret_stays_visible_when_the_text_is_long() {
        let (mut gui, id) = with_field(TextField::new().min_width(80.0));
        laid_out(&mut gui);
        gui.set_focus(id);

        let width = gui.tree().bounds(id).width - PAD * 2.0;

        // 枠より長く打つ。
        for _ in 0..40 {
            type_text(&mut gui, "x");
            laid_out(&mut gui);
        }

        let text_field = field(&gui, id);

        assert!(text_field.scroll() > 0.0, "送っている");

        // キャレットは枠の中。
        let caret = text_field.caret_rect();
        assert!(caret.x >= text_field.inner.x - 0.01, "{caret:?}");
        assert!(caret.right() <= text_field.inner.x + width + 0.01, "{caret:?}");
    }

    #[test]
    fn home_scrolls_back_to_the_start() {
        let (mut gui, id) = with_field(TextField::with_text("x".repeat(40)).min_width(80.0));
        laid_out(&mut gui);
        gui.set_focus(id);

        // 末尾にキャレットがあるので送られている。
        press(&mut gui, NamedKey::End);
        laid_out(&mut gui);
        assert!(field(&gui, id).scroll() > 0.0);

        press(&mut gui, NamedKey::Home);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).scroll(), 0.0);
        assert_eq!(field(&gui, id).caret(), 0);
    }

    #[test]
    fn a_short_text_is_never_scrolled() {
        let (mut gui, id) = with_field(TextField::with_text("ab"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press(&mut gui, NamedKey::End);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).scroll(), 0.0, "右に空白を作らない");
    }

    // --- 見た目 ---

    #[test]
    fn the_text_is_clipped_to_the_inside() {
        let (mut gui, id) = with_field(TextField::with_text("x".repeat(40)).min_width(80.0));
        laid_out(&mut gui);

        let list = gui.paint();

        // 背景は切り抜かない。字は切り抜く。
        let background = &list.commands()[0];
        assert!(background.clip.is_none());

        let text = list
            .commands()
            .iter()
            .find(|command| matches!(command.primitive, Primitive::Text { .. }))
            .expect("字");

        let clip = text.clip.expect("字は切り抜かれる");
        let bounds = gui.tree().bounds(id);

        assert_eq!(clip.rect, bounds.deflate(Insets::all(PAD)));
    }

    #[test]
    fn the_caret_shows_only_when_focused() {
        let (mut gui, id) = with_field(TextField::with_text("ab"));
        laid_out(&mut gui);

        let caret_count = |gui: &mut Gui| {
            let expected = field(gui, id).caret_rect();
            let list = gui.paint();

            list.commands()
                .iter()
                .filter(|command| command.primitive.bounds() == expected)
                .count()
        };

        assert_eq!(caret_count(&mut gui), 0, "焦点が無ければ出さない");

        gui.set_focus(id);
        assert_eq!(caret_count(&mut gui), 1);
    }

    #[test]
    fn the_selection_is_painted_under_the_text() {
        let (mut gui, id) = with_field(TextField::with_text("abcd"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.tree_mut().get_as_mut::<TextField>(id).unwrap().select_all();

        let list = gui.paint();
        let kinds: Vec<&str> = list
            .commands()
            .iter()
            .map(|command| match command.primitive {
                Primitive::Rect { .. } => "rect",
                Primitive::Text { .. } => "text",
                _ => "other",
            })
            .collect();

        // 背景 → 選んだ範囲 → 字 → キャレット。
        assert_eq!(kinds, vec!["rect", "rect", "text", "rect"], "{kinds:?}");

        let selection = field(&gui, id).selection_rect().expect("選んでいる");
        assert_eq!(selection.width, ADVANCE * 4.0);
    }

    #[test]
    fn the_placeholder_shows_only_when_empty() {
        let (mut gui, id) = with_field(TextField::new().placeholder("名前"));
        laid_out(&mut gui);

        let shown = |gui: &mut Gui| {
            let list = gui.paint();

            list.commands()
                .iter()
                .filter_map(|command| match &command.primitive {
                    Primitive::Text { text, color, .. } => Some((text.clone(), *color)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        let hint = shown(&mut gui);
        assert_eq!(hint.len(), 1);
        assert_eq!(hint[0].0, "名前");
        assert_eq!(hint[0].1, Color::hex(0x666666), "案内の役から色を引く");

        gui.set_focus(id);
        type_text(&mut gui, "a");
        laid_out(&mut gui);

        let typed = shown(&mut gui);
        assert_eq!(typed[0].0, "a", "打ったら消える");
        assert_eq!(typed[0].1, Color::WHITE);
    }

    #[test]
    fn a_mask_hides_the_text_but_not_the_value() {
        let (mut gui, id) = with_field(TextField::with_text("secret").mask('*'));
        laid_out(&mut gui);

        let list = gui.paint();
        let shown = list
            .commands()
            .iter()
            .find_map(|command| match &command.primitive {
                Primitive::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .expect("字");

        assert_eq!(shown, "******");
        assert_eq!(field(&gui, id).text(), "secret", "素の字は残る");
    }

    #[test]
    fn a_masked_field_still_maps_clicks_to_the_real_bytes() {
        let (mut gui, id) = with_field(TextField::with_text("あい").mask('*'));
        laid_out(&mut gui);

        let bounds = gui.tree().bounds(id);

        // 隠し字は 1 バイト、素の字は 3 バイト。表が食い違っていると壊れる。
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(bounds.x + PAD + ADVANCE, bounds.center().y),
            button: PointerButton::Primary,
        });

        let caret = field(&gui, id).caret();

        assert_eq!(caret, 3, "1 文字目の後ろ = 3 バイト目");
        assert!(field(&gui, id).text().is_char_boundary(caret));
    }

    // --- その他 ---

    #[test]
    fn losing_focus_drops_the_selection() {
        let (mut gui, id) = with_field(TextField::with_text("abc"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.tree_mut().get_as_mut::<TextField>(id).unwrap().select_all();
        assert!(field(&gui, id).has_selection());

        gui.set_focus(WidgetId::NONE);

        assert!(!field(&gui, id).has_selection());
        assert_eq!(field(&gui, id).text(), "abc", "字は消えない");
    }

    #[test]
    fn a_disabled_field_takes_nothing() {
        let (mut gui, id) = with_field(TextField::with_text("abc"));
        laid_out(&mut gui);

        gui.tree_mut().set_disabled(id, true);
        gui.set_focus(id);

        type_text(&mut gui, "x");
        press(&mut gui, NamedKey::Backspace);

        assert_eq!(field(&gui, id).text(), "abc");
        assert_eq!(gui.hit_test(Point::new(10.0, 10.0)), WidgetId::NONE);
    }

    #[test]
    fn tab_leaves_the_field_alone() {
        let (mut gui, id) = with_field(TextField::with_text("abc"));
        laid_out(&mut gui);
        gui.set_focus(id);

        press(&mut gui, NamedKey::Tab);

        // Tab は焦点の移動に使うので、欄は受け取らない。
        assert_eq!(field(&gui, id).text(), "abc");
        assert!(gui.drain_actions().is_empty());
    }

    #[test]
    fn set_text_moves_the_caret_to_the_end() {
        let (mut gui, id) = with_field(TextField::new());
        laid_out(&mut gui);

        gui.tree_mut()
            .get_as_mut::<TextField>(id)
            .unwrap()
            .set_text("hello");
        gui.tree_mut().request_layout(id);
        laid_out(&mut gui);

        assert_eq!(field(&gui, id).caret(), 5);
        assert_eq!(field(&gui, id).offset_of(5), ADVANCE * 5.0);
    }

    #[test]
    fn two_fields_hold_their_own_text() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let first = gui.tree_mut().add_child(root, Box::new(TextField::new()));
        let second = gui.tree_mut().add_child(root, Box::new(TextField::new()));

        laid_out(&mut gui);

        gui.set_focus(first);
        type_text(&mut gui, "one");
        laid_out(&mut gui);

        gui.set_focus(second);
        type_text(&mut gui, "two");
        laid_out(&mut gui);

        assert_eq!(field(&gui, first).text(), "one");
        assert_eq!(field(&gui, second).text(), "two");

        // Tab で行き来できる。
        gui.focus_next();
        assert_eq!(gui.focused(), first);
    }
}
