//! 周回のあいだウィジェットに渡す道具。
//!
//! 4 つあり、**できることが周回ごとに違います**。できないことを型で
//! 封じておくと、周回の順番を壊す書き方が通らなくなります。
//!
//! | | 子を測る | 子を置く | 描く | 状態を変える |
//! |---|---|---|---|---|
//! | [`MeasureContext`] | できる | | | |
//! | [`ArrangeContext`] | できる | できる | | |
//! | [`PaintContext`] | | | できる | |
//! | [`EventContext`] | | | | できる |
//!
//! 描く周回で状態を変えられないのは、**描く順によって結果が変わってしまう**
//! からです。状態は入力の周回でだけ動きます。

use crate::font::Font;
use crate::gui::event::Modifiers;
use crate::gui::geometry::{Point, Rect, Size};
use crate::gui::id::{Tag, WidgetId};
use crate::gui::layout::Constraints;
use crate::gui::painter::TextLayoutOptions;
use crate::gui::theme::{Metrics, Role, Style, Theme};
use crate::gui::widget::{ActionKind, WidgetState};
use crate::gui::Gui;
use crate::text::{TextStyle, measure, measure_wrapped};

/// 文字の寸法を答えるもの。
///
/// # なぜ [`Gui`] が書体を持たないのか
///
/// [`Font`] は読み込んだバイト列を借りているので、持たせると [`Gui`] に
/// 寿命が付きます。寿命の付いた構造体はフレームをまたいで持ち回すのが面倒で、
/// 「UI を作る道具」としては割に合いません。
///
/// 代わりに**測る周回のあいだだけ借ります**。
/// [`Gui::layout`](crate::gui::Gui::layout) に渡してください。
pub trait TextMeasure {
    /// その文字列が要る大きさ。`max_width` があればそこで折り返す。
    fn measure_text(
        &self,
        text: &str,
        options: TextLayoutOptions,
        max_width: Option<f32>,
    ) -> Size;

    /// 1 行ぶんの高さ。送り量を画素に直すときにも使う。
    fn line_height(&self, text_size: f32) -> f32 {
        text_size * 1.4
    }
}

/// 書体が無いときの代わり。**何を測っても 0 を返します。**
///
/// 文字を出さない画面を組むときや、書体を読む前のフレームに使います。
/// 文字のあるウィジェットは大きさ 0 として扱われるので、
/// 「文字が出ない」ではなく「場所が取れていない」として見えます。
pub struct NoTextMeasure;

impl TextMeasure for NoTextMeasure {
    fn measure_text(&self, _text: &str, _options: TextLayoutOptions, _max: Option<f32>) -> Size {
        Size::ZERO
    }
}

/// [`Font`] で測る。
pub struct FontMeasure<'a> {
    font: &'a Font<'a>,
}

impl<'a> FontMeasure<'a> {
    pub fn new(font: &'a Font<'a>) -> Self {
        Self { font }
    }
}

impl TextMeasure for FontMeasure<'_> {
    fn measure_text(
        &self,
        text: &str,
        options: TextLayoutOptions,
        max_width: Option<f32>,
    ) -> Size {
        let style = TextStyle::new(options.size);

        // 折り返すかは呼ぶ側の指定と、幅が来ているかの両方で決まる。
        let size = match max_width.filter(|_| options.wrap) {
            Some(max_width) => measure_wrapped(self.font, text, &style, max_width),
            None => measure(self.font, text, &style),
        };

        // 塗られる範囲ではなく送り幅で測る。場所取りは送り幅で揃えないと、
        // 斜体や影のある行だけ余計に広くなる。
        Size::new(size.width, size.height)
    }

    fn line_height(&self, text_size: f32) -> f32 {
        self.font.line_height() * text_size
    }
}

/// 測る周回。
pub struct MeasureContext<'a> {
    pub(crate) gui: &'a mut Gui,
    pub(crate) id: WidgetId,
    pub(crate) text: &'a dyn TextMeasure,
}

impl MeasureContext<'_> {
    /// 自分の鍵。
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// 子の鍵。順番は足した順。
    pub fn children(&self) -> Vec<WidgetId> {
        self.gui.tree().children(self.id).to_vec()
    }

    pub fn child_count(&self) -> usize {
        self.gui.tree().children(self.id).len()
    }

    /// 子を測る。同じ制約で 2 度呼んでも、2 度目は覚えから返るので安い。
    pub fn measure_child(&mut self, child: WidgetId, constraints: Constraints) -> Size {
        self.gui.measure_subtree(child, constraints, self.text)
    }

    /// 文字の寸法。
    pub fn measure_text(
        &self,
        text: &str,
        options: TextLayoutOptions,
        max_width: Option<f32>,
    ) -> Size {
        self.text.measure_text(text, options, max_width)
    }

    pub fn line_height(&self, text_size: f32) -> f32 {
        self.text.line_height(text_size)
    }

    pub fn state(&self) -> WidgetState {
        self.gui.tree().state(self.id)
    }

    pub fn theme(&self) -> &Theme {
        self.gui.theme()
    }

    pub fn metrics(&self) -> &Metrics {
        self.gui.theme().metrics()
    }

    /// 自分の役での見た目。余白を測りに入れるときに使う。
    pub fn style(&self, role: Role) -> Style {
        self.gui.theme().style(role, self.state())
    }
}

/// 置く周回。
pub struct ArrangeContext<'a> {
    pub(crate) gui: &'a mut Gui,
    pub(crate) id: WidgetId,
    /// 自分の画面上の左上。[`ArrangeContext::place`] がこれを足す。
    pub(crate) origin: Point,
    pub(crate) text: &'a dyn TextMeasure,
    pub(crate) placements: Vec<(WidgetId, Rect)>,
}

impl ArrangeContext<'_> {
    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn children(&self) -> Vec<WidgetId> {
        self.gui.tree().children(self.id).to_vec()
    }

    pub fn child_count(&self) -> usize {
        self.gui.tree().children(self.id).len()
    }

    /// 子の置き場所を決める。`rect` は**自分の左上を原点とした**矩形。
    ///
    /// 置かれた子の中身は、この周回が終わってから順に置かれます。
    /// 同じ子に 2 度置くと**後のほうが勝ちます**。
    pub fn place(&mut self, child: WidgetId, rect: Rect) {
        if !self.gui.tree().is_valid(child) {
            return;
        }

        let global = rect.translate(self.origin);

        if let Some(slot) = self
            .placements
            .iter_mut()
            .find(|(placed, _)| *placed == child)
        {
            slot.1 = global;
            return;
        }

        self.placements.push((child, global));
    }

    /// 直近に測った子の「欲しい大きさ」。測っていなければ 0。
    pub fn desired(&self, child: WidgetId) -> Size {
        self.gui
            .tree()
            .get(child)
            .map(|node| node.desired())
            .unwrap_or(Size::ZERO)
    }

    /// 子を測る。置く前に大きさが要るとき。覚えがあればそこから返る。
    pub fn measure_child(&mut self, child: WidgetId, constraints: Constraints) -> Size {
        self.gui.measure_subtree(child, constraints, self.text)
    }

    /// 画面の大きさ。
    ///
    /// # これを見てよいのは浮かせるものだけ
    ///
    /// ふつうのウィジェットが画面の大きさで形を変えると、同じものを
    /// 別の場所に置いたときに違う結果になります。**使ってよいのは、
    /// 下に出すか上に出すかを決めるような「画面の端に当たるか」の判断だけです。**
    pub fn viewport(&self) -> Size {
        self.gui.viewport()
    }

    /// 自分の画面上の左上。
    ///
    /// # これを見てよいのは浮かせるものだけ
    ///
    /// [`ArrangeContext::viewport`] と同じ理由です。開いた一覧が画面の下から
    /// はみ出すかを見るには、自分が画面のどこに居るかを知る必要があります。
    /// それ以外で使うと、置き場所によって形が変わるウィジェットになります。
    pub fn screen_origin(&self) -> Point {
        self.origin
    }

    /// 子が自分で置き場所を決めているなら、その左上。
    ///
    /// [`Widget::absolute_position`](crate::gui::Widget::absolute_position) の値です。
    /// 浮かせる箱（[`Floating`](crate::gui::container::Floating)）だけが見ます。
    pub fn absolute_position(&self, child: WidgetId) -> Option<Point> {
        self.gui.tree().get(child)?.widget()?.absolute_position()
    }

    pub fn measure_text(
        &self,
        text: &str,
        options: TextLayoutOptions,
        max_width: Option<f32>,
    ) -> Size {
        self.text.measure_text(text, options, max_width)
    }

    pub fn line_height(&self, text_size: f32) -> f32 {
        self.text.line_height(text_size)
    }

    pub fn state(&self) -> WidgetState {
        self.gui.tree().state(self.id)
    }

    pub fn theme(&self) -> &Theme {
        self.gui.theme()
    }

    pub fn metrics(&self) -> &Metrics {
        self.gui.theme().metrics()
    }

    pub fn style(&self, role: Role) -> Style {
        self.gui.theme().style(role, self.state())
    }
}

/// 描く周回。**読むだけ。**
pub struct PaintContext<'a> {
    pub(crate) gui: &'a Gui,
    pub(crate) id: WidgetId,
}

impl PaintContext<'_> {
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// 自分の大きさ。左上は常に `(0, 0)` なので、矩形は
    /// `Rect::from_origin_size(Point::ZERO, self.size())`。
    pub fn size(&self) -> Size {
        self.gui.tree().bounds(self.id).size()
    }

    /// 自分の局所矩形。
    pub fn rect(&self) -> Rect {
        Rect::from_origin_size(Point::ZERO, self.size())
    }

    pub fn children(&self) -> &[WidgetId] {
        self.gui.tree().children(self.id)
    }

    pub fn state(&self) -> WidgetState {
        self.gui.tree().state(self.id)
    }

    pub fn theme(&self) -> &Theme {
        self.gui.theme()
    }

    pub fn metrics(&self) -> &Metrics {
        self.gui.theme().metrics()
    }

    /// いまの状態での見た目。デザイン側がいちばん使う入口。
    pub fn style(&self, role: Role) -> Style {
        self.gui.theme().style(role, self.state())
    }

    /// 子の大きさ。自分で子の見た目まで描きたいときに。
    pub fn child_size(&self, child: WidgetId) -> Size {
        self.gui.tree().bounds(child).size()
    }

    /// 子の、自分から見た位置。
    pub fn child_rect(&self, child: WidgetId) -> Rect {
        let origin = self.gui.tree().bounds(self.id).origin();

        self.gui
            .tree()
            .bounds(child)
            .translate(Point::new(-origin.x, -origin.y))
    }
}

/// 入力の周回。**状態を動かせる唯一の周回。**
pub struct EventContext<'a> {
    pub(crate) gui: &'a mut Gui,
    pub(crate) id: WidgetId,
}

impl EventContext<'_> {
    pub fn id(&self) -> WidgetId {
        self.id
    }

    pub fn size(&self) -> Size {
        self.gui.tree().bounds(self.id).size()
    }

    pub fn rect(&self) -> Rect {
        Rect::from_origin_size(Point::ZERO, self.size())
    }

    pub fn children(&self) -> Vec<WidgetId> {
        self.gui.tree().children(self.id).to_vec()
    }

    pub fn state(&self) -> WidgetState {
        self.gui.tree().state(self.id)
    }

    pub fn theme(&self) -> &Theme {
        self.gui.theme()
    }

    pub fn metrics(&self) -> &Metrics {
        self.gui.theme().metrics()
    }

    /// いま押されている修飾鍵。
    pub fn modifiers(&self) -> Modifiers {
        self.gui.modifiers()
    }

    /// ポインタの画面上の位置。窓の外なら `None`。
    pub fn pointer(&self) -> Option<Point> {
        self.gui.pointer()
    }

    /// ポインタの、自分から見た位置。
    pub fn local_pointer(&self) -> Option<Point> {
        let origin = self.gui.tree().bounds(self.id).origin();

        self.gui.pointer().map(|point| point - origin)
    }

    /// 呼ぶ側へ伝える。[`Gui::drain_actions`](crate::gui::Gui::drain_actions) で届く。
    pub fn emit(&mut self, kind: ActionKind) {
        self.gui.push_action(self.id, kind);
    }

    /// 形が変わったので測り直してほしい。
    ///
    /// **大きさに関わる値を書き換えたら必ず呼んでください。** 呼ばないと
    /// 前のフレームの大きさのまま描かれます。
    pub fn request_layout(&mut self) {
        let id = self.id;
        self.gui.tree_mut().request_layout(id);
    }

    /// 焦点をもらう。[`Behavior::focusable`](crate::gui::widget::Behavior::focusable)
    /// が立っていないと何も起きません。
    pub fn request_focus(&mut self) {
        let id = self.id;
        self.gui.set_focus(id);
    }

    /// 焦点を手放す。自分が持っていなければ何も起きません。
    pub fn release_focus(&mut self) {
        if self.gui.focused() == self.id {
            self.gui.set_focus(WidgetId::NONE);
        }
    }

    /// ポインタを掴む。放すまで、枠の外へ出ても入力が自分に届きます。
    pub fn capture_pointer(&mut self) {
        let id = self.id;
        self.gui.set_pointer_capture(id);
    }

    /// 掴みを放す。
    pub fn release_pointer(&mut self) {
        if self.gui.pointer_capture() == self.id {
            self.gui.set_pointer_capture(WidgetId::NONE);
        }
    }

    /// 自分が掴んでいるか。
    pub fn has_pointer_capture(&self) -> bool {
        self.gui.pointer_capture() == self.id
    }

    /// 札を引いて別のウィジェットを探す。
    pub fn find(&self, tag: Tag) -> Option<WidgetId> {
        self.gui.tree().find(tag)
    }
}
