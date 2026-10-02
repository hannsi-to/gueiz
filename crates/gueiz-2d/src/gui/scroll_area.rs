//! はみ出した中身を、送って見せる箱。
//!
//! # 送るのは描くときではなく、置くとき
//!
//! [`Painter::with_offset`](crate::gui::painter::Painter::with_offset) で
//! ずらして描くこともできますが、**それだと当たり判定がずれます。**
//! 描く位置だけ動いて、木に入っている矩形は動かないからです。
//!
//! そこで [`ScrollArea`] は `arrange` の周回で**中身を負の位置に置きます**。
//!
//! ```text
//! offset = 40 のとき
//!
//!   木に入る矩形            画面
//!   ┌─────────┐ y = -40
//!   │ 見えない  │
//!   ├─────────┤ y = 0     ┌─────────┐
//!   │         │           │         │
//!   │  中身   │           │  中身   │  ← clips_children で切る
//!   │         │           │         │
//!   ├─────────┤           └─────────┘
//!   │ 見えない  │
//!   └─────────┘
//! ```
//!
//! 置き場所そのものが動くので、**当たり判定も焦点も自動で付いてきます。**
//! [`Behavior::clips_children`] が立っているので、枠の外へ出た子は
//! [`Gui::hit_test`](crate::gui::Gui::hit_test) でも当たりません。
//!
//! # 端まで行ったら親に渡す
//!
//! [`Event::Scrolled`] を受けて**実際に動いたときだけ**
//! [`EventResult::Consumed`] を返します。端で止まっているときは返さないので、
//! 入れ子にした外側のスクロール領域が続きを受け取ります。
//!
//! これをせずに常に受け取ると、内側の一覧が端に着いた時点で
//! 画面全体のスクロールが死にます。
//!
//! # 軸方向の上限が無いと、送りません
//!
//! 枠の丈が決まっていなければ「はみ出す」という状態がありません。
//! 軸方向の上限が無い制約を渡されたとき、[`ScrollArea`] は
//! **中身の丈そのままの大きさになり、スクロールしなくなります。**
//!
//! これを踏むのは、**スクロール領域を別のスクロール領域に直に入れたとき**です。
//! 外側は中身を上限なしで測るので、内側には上限が届きません。
//!
//! ```text
//! ScrollArea ──▶ ScrollArea          内側は送らない（上限が無い）
//! ScrollArea ──▶ Stack ──▶ ScrollArea  内側に Length::Fixed を敷けば送る
//! ```
//!
//! 内側にも送らせたいなら、[`Stack::set_length`](crate::gui::container::Stack::set_length)
//! で丈を決めてください。
//!
//! # 1 つの軸だけ
//!
//! [`ScrollArea`] は [`Axis`] 1 つぶんです。縦横どちらも送りたいなら、
//! **横の中に縦を入れてください。** 棒が 2 本になる作りは、棒の交わる隅の
//! 扱いだけで中身が倍になるので、そこは呼ぶ側の組み合わせに任せています。
//!
//! # 棒の幅は先に取っておく
//!
//! 「中身が収まるかどうか」で棒を出し入れすると、**出した瞬間に幅が狭まって
//! 中身が伸び、また収まらなくなる**という循環に入ります。
//!
//! なので [`ScrollArea::scrollbar_width`] が 0 でなければ、
//! **収まっていても幅を取っておきます。** 棒そのものは、送る必要が
//! 無ければ描きません。幅を節約したいなら `0.0` にして、棒なしで使ってください。
//!
//! # 見た目
//!
//! 棒は [`ScrollArea::scrollbar_role`]（既定は [`Role::ACCENT`]）から引きます。
//!
//! | [`Style`] の欄 | 何に使うか |
//! |---|---|
//! | `background` | 棒の通り道 |
//! | `foreground` | つまみ |
//! | `corners` | つまみの丸み |
//!
//! つまみに乗っている・掴んでいるあいだは、`Hovered` / `Pressed` の型に
//! 切り替わります。**領域そのものの状態ではなく、つまみの状態です。**
//!
//! [`Behavior::clips_children`]: crate::gui::widget::Behavior::clips_children
//! [`Event::Scrolled`]: crate::gui::event::Event::Scrolled
//! [`Style`]: crate::gui::theme::Style

use crate::gui::event::{Event, EventResult, PointerButton};
use crate::gui::geometry::{Axis, Point, Rect, Size};
use crate::gui::layout::Constraints;
use crate::gui::painter::Painter;
use crate::gui::theme::Role;
use crate::gui::widget::{Behavior, WidgetState};
use crate::gui::{ArrangeContext, EventContext, MeasureContext, PaintContext, Widget};

/// つまみがこれより短くならないようにする。短すぎると掴めない。
const DEFAULT_MIN_THUMB: f32 = 24.0;

/// 棒の既定の幅。
const DEFAULT_SCROLLBAR_WIDTH: f32 = 10.0;

/// はみ出した中身を送って見せる箱。
pub struct ScrollArea {
    axis: Axis,
    /// 送った量。0 が先頭。**常に 0 以上。**
    offset: f32,

    scrollbar_role: Role,
    scrollbar_width: f32,
    min_thumb: f32,

    /// 中身を軸と直交する向きいっぱいに伸ばすか。
    stretch_cross: bool,
    /// 車輪 1 刻みで送る量。`None` なら行の高さぶん。
    step: Option<f32>,

    // --- 測った結果。`measure` と `arrange` が書く ---
    /// 自分の大きさ。
    viewport: Size,
    /// 中身の軸方向の丈。
    content: f32,

    // --- つまみの状態 ---
    thumb_hovered: bool,
    /// 掴んでいるあいだ、つまみの頭からどれだけ奥を掴んだか。
    thumb_grab: Option<f32>,
}

impl ScrollArea {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            offset: 0.0,
            scrollbar_role: Role::ACCENT,
            scrollbar_width: DEFAULT_SCROLLBAR_WIDTH,
            min_thumb: DEFAULT_MIN_THUMB,
            stretch_cross: true,
            step: None,
            viewport: Size::ZERO,
            content: 0.0,
            thumb_hovered: false,
            thumb_grab: None,
        }
    }

    /// 縦に送る。
    pub fn vertical() -> Self {
        Self::new(Axis::Vertical)
    }

    /// 横に送る。
    pub fn horizontal() -> Self {
        Self::new(Axis::Horizontal)
    }

    pub fn scrollbar_role(mut self, scrollbar_role: Role) -> Self {
        self.scrollbar_role = scrollbar_role;
        self
    }

    /// 棒の幅。`0.0` で棒なし（送りは車輪だけになります）。
    pub fn scrollbar_width(mut self, scrollbar_width: f32) -> Self {
        self.scrollbar_width = scrollbar_width.max(0.0);
        self
    }

    /// つまみの最小の長さ。
    pub fn min_thumb(mut self, min_thumb: f32) -> Self {
        self.min_thumb = min_thumb.max(1.0);
        self
    }

    /// 中身を軸と直交する向きいっぱいに伸ばすか。既定は伸ばす。
    ///
    /// 倒すと、中身は欲しいだけの幅になります。横にもはみ出すときは
    /// 横のスクロール領域で包んでください。
    pub fn stretch_cross(mut self, stretch_cross: bool) -> Self {
        self.stretch_cross = stretch_cross;
        self
    }

    /// 車輪 1 刻みで送る量。既定は
    /// [`Metrics::line_height`](crate::gui::theme::Metrics) ぶん。
    pub fn step(mut self, step: f32) -> Self {
        self.step = Some(step.max(0.0));
        self
    }

    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// いま送っている量。
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// 送れるいちばん奥。中身が収まっていれば 0。
    pub fn max_offset(&self) -> f32 {
        (self.content - self.viewport_length()).max(0.0)
    }

    /// 中身の丈。
    pub fn content_length(&self) -> f32 {
        self.content
    }

    /// 送る必要があるか。
    pub fn is_scrollable(&self) -> bool {
        self.max_offset() > 0.0
    }

    /// つまみを掴んでいるか。
    pub fn is_dragging(&self) -> bool {
        self.thumb_grab.is_some()
    }

    /// 送る量を決める。範囲外は丸められます。
    ///
    /// **呼んだら
    /// [`WidgetTree::request_layout`](crate::gui::tree::WidgetTree::request_layout)
    /// も呼んでください。**中身の置き場所が変わるので、測り直しが要ります。
    ///
    /// ```no_run
    /// # use gueiz_2d::gui::id::WidgetId;
    /// # use gueiz_2d::gui::scroll_area::ScrollArea;
    /// # use gueiz_2d::gui::Gui;
    /// # fn to_bottom(gui: &mut Gui, id: WidgetId) {
    /// if let Some(area) = gui.tree_mut().get_as_mut::<ScrollArea>(id) {
    ///     area.scroll_to_end();
    /// }
    ///
    /// gui.tree_mut().request_layout(id);
    /// # }
    /// ```
    pub fn set_offset(&mut self, offset: f32) -> bool {
        let clamped = if offset.is_finite() {
            offset.clamp(0.0, self.max_offset())
        } else {
            0.0
        };

        let moved = clamped != self.offset;
        self.offset = clamped;

        moved
    }

    /// いまの位置から送る。動いたら `true`。
    pub fn scroll_by(&mut self, amount: f32) -> bool {
        self.set_offset(self.offset + amount)
    }

    /// 先頭へ。
    pub fn scroll_to_start(&mut self) -> bool {
        self.set_offset(0.0)
    }

    /// 末尾へ。記録を流すような画面で使います。
    pub fn scroll_to_end(&mut self) -> bool {
        self.set_offset(self.max_offset())
    }

    /// その範囲が見えるところまで送る。`rect` は**中身の座標**（送る前の位置）。
    ///
    /// すでに見えていれば動きません。
    pub fn scroll_into_view(&mut self, rect: Rect) -> bool {
        let (near, far) = match self.axis {
            Axis::Vertical => (rect.y, rect.bottom()),
            Axis::Horizontal => (rect.x, rect.right()),
        };

        let viewport = self.viewport_length();

        // 手前が隠れているなら手前へ、奥が隠れているなら奥へ。
        // 両方隠れている（枠より長い）ときは手前を優先する。
        if near < self.offset {
            self.set_offset(near)
        } else if far > self.offset + viewport {
            self.set_offset(far - viewport)
        } else {
            false
        }
    }

    /// 軸方向の枠の長さ。
    fn viewport_length(&self) -> f32 {
        self.viewport.along(self.axis)
    }

    /// 棒のために取っておく幅。棒を出さない設定なら 0。
    fn reserved_bar(&self) -> f32 {
        self.scrollbar_width
    }

    /// 中身に渡せる、軸と直交する向きの長さ。
    fn content_cross(&self) -> f32 {
        (self.viewport.across(self.axis) - self.reserved_bar()).max(0.0)
    }

    /// 棒の通り道。局所座標。棒なしなら空。
    pub fn track(&self) -> Rect {
        if self.scrollbar_width <= 0.0 {
            return Rect::ZERO;
        }

        match self.axis {
            Axis::Vertical => Rect::new(
                self.viewport.width - self.scrollbar_width,
                0.0,
                self.scrollbar_width,
                self.viewport.height,
            ),
            Axis::Horizontal => Rect::new(
                0.0,
                self.viewport.height - self.scrollbar_width,
                self.viewport.width,
                self.scrollbar_width,
            ),
        }
    }

    /// つまみ。局所座標。送る必要が無ければ `None`。
    pub fn thumb(&self) -> Option<Rect> {
        let track = self.track();

        if track.is_empty() || !self.is_scrollable() {
            return None;
        }

        let track_length = track.size().along(self.axis);
        let viewport = self.viewport_length();

        // 枠と中身の比がつまみの長さ。短すぎると掴めないので下限を敷く。
        let length = (track_length * viewport / self.content)
            .max(self.min_thumb)
            .min(track_length);

        let travel = track_length - length;
        let position = if travel > 0.0 {
            travel * self.offset / self.max_offset()
        } else {
            0.0
        };

        Some(match self.axis {
            Axis::Vertical => Rect::new(track.x, track.y + position, track.width, length),
            Axis::Horizontal => Rect::new(track.x + position, track.y, length, track.height),
        })
    }

    /// 棒の上の 1 ピクセルが、送る量の何ピクセルにあたるか。
    fn offset_per_pixel(&self) -> f32 {
        let Some(thumb) = self.thumb() else {
            return 0.0;
        };

        let travel = self.track().size().along(self.axis) - thumb.size().along(self.axis);

        if travel <= 0.0 {
            return 0.0;
        }

        self.max_offset() / travel
    }

    /// つまみの見た目を引くときの状態。
    ///
    /// 領域そのものの状態ではなく、**つまみ**の状態を返す。
    fn thumb_state(&self, state: WidgetState) -> WidgetState {
        WidgetState {
            hovered: self.thumb_hovered,
            pressed: self.thumb_grab.is_some(),
            focused: false,
            disabled: state.disabled,
        }
    }

    /// 局所座標の軸方向の成分。
    fn along(&self, point: Point) -> f32 {
        match self.axis {
            Axis::Vertical => point.y,
            Axis::Horizontal => point.x,
        }
    }
}

impl Widget for ScrollArea {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let bar = self.reserved_bar();

        // 直交する向きに中身へ渡せる長さ。上限が無ければそのまま無限。
        let cross_available = constraints.max.across(self.axis);
        let cross_for_content = if cross_available.is_finite() {
            (cross_available - bar).max(0.0)
        } else {
            f32::INFINITY
        };

        // 軸方向は**上限なし**で聞く。これが要点。枠の丈で縛ると、
        // 中身が自分で縮んでしまい、送るものが無くなる。
        let child_constraints = Constraints {
            min: if self.stretch_cross && cross_for_content.is_finite() {
                Size::from_axis(self.axis, 0.0, cross_for_content)
            } else {
                Size::ZERO
            },
            max: Size::from_axis(self.axis, f32::INFINITY, cross_for_content),
        };

        let mut content = 0.0_f32;
        let mut content_cross = 0.0_f32;

        for child in context.children() {
            let size = context.measure_child(child, child_constraints);

            // 子は軸に沿って積む。間隔や寄せが要るなら中に `Stack` を置く。
            content += size.along(self.axis);
            content_cross = content_cross.max(size.across(self.axis));
        }

        self.content = content;

        // 自分の大きさ。軸方向は枠いっぱい。上限が無いなら送る意味がないので
        // 中身の丈そのまま。
        let along = if constraints.has_bounded(self.axis) {
            constraints.max.along(self.axis)
        } else {
            content
        };

        let cross = if cross_available.is_finite() {
            cross_available
        } else {
            content_cross + bar
        };

        constraints.constrain(Size::from_axis(self.axis, along, cross))
    }

    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        self.viewport = bounds.size();

        // 中身が縮んだり枠が広がったりして、送りすぎになっていることがある。
        self.offset = self.offset.clamp(0.0, self.max_offset());

        let cross_limit = self.content_cross();
        let mut along = -self.offset;

        for child in context.children() {
            let desired = context.desired(child);
            let length = desired.along(self.axis);

            // 伸ばす設定なら枠いっぱい、そうでなければ中身の欲しいぶん。
            // 欲しいぶんのほうが広いときは**切らずに置く**。横にはみ出したものは
            // 横のスクロール領域で包んで送るのが筋なので、ここで潰さない。
            let cross = if self.stretch_cross {
                cross_limit
            } else {
                desired.across(self.axis)
            };

            let rect = match self.axis {
                Axis::Vertical => Rect::new(0.0, along, cross, length),
                Axis::Horizontal => Rect::new(along, 0.0, length, cross),
            };

            context.place(child, rect);
            along += length;
        }
    }

    /// 棒は中身の**上**に乗せる。先に描くと中身で隠れる。
    fn paint_foreground(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let Some(thumb) = self.thumb() else {
            // 送る必要が無いときは通り道も描かない。
            // 使えない棒が出ていると、送れるように見えてしまう。
            return;
        };

        let style = context
            .theme()
            .style(self.scrollbar_role, self.thumb_state(context.state()));

        painter.rect(self.track(), style.background);
        painter.rounded_rect(thumb, style.corners, style.foreground);
    }

    fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
        match event {
            Event::Scrolled { delta, .. } => {
                let step = self
                    .step
                    .unwrap_or_else(|| context.metrics().line_height(context.metrics().text_size));

                let pixels = delta.to_pixels(step);
                let amount = self.along(pixels);

                // 送りの向き: 正は「中身が手前へ来る」なので、offset は減る。
                if !self.scroll_by(-amount) {
                    // 端で止まった。受け取らずに外側へ渡す。
                    return EventResult::Ignored;
                }

                context.request_layout();

                EventResult::Consumed
            }

            Event::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                let Some(thumb) = self.thumb() else {
                    return EventResult::Ignored;
                };

                if thumb.contains(*position) {
                    // つまみを掴んだ。頭からどれだけ奥かを覚えておく。
                    self.thumb_grab = Some(self.along(*position) - self.along(thumb.origin()));
                    context.capture_pointer();

                    return EventResult::Consumed;
                }

                if self.track().contains(*position) {
                    // 通り道を押した。1 枠ぶん飛ばす。
                    let viewport = self.viewport_length();
                    let forward = self.along(*position) > self.along(thumb.origin());

                    if self.scroll_by(if forward { viewport } else { -viewport }) {
                        context.request_layout();
                    }

                    return EventResult::Consumed;
                }

                // 棒の外。中身が取らなかったぶんなので、外側へ渡す。
                EventResult::Ignored
            }

            Event::PointerMove { position, delta } => {
                self.thumb_hovered = self
                    .thumb()
                    .is_some_and(|thumb| thumb.contains(*position));

                if self.thumb_grab.is_none() {
                    return EventResult::Ignored;
                }

                // 棒の上を動いたぶんを、送る量に換算する。
                let moved = self.along(*delta) * self.offset_per_pixel();

                if self.scroll_by(moved) {
                    context.request_layout();
                }

                EventResult::Consumed
            }

            Event::PointerReleased { .. } if self.thumb_grab.is_some() => {
                self.thumb_grab = None;
                context.release_pointer();

                EventResult::Consumed
            }

            Event::PointerExit => {
                self.thumb_hovered = false;
                EventResult::Ignored
            }

            _ => EventResult::Ignored,
        }
    }

    /// 棒の上だけ、当たりを取る。
    ///
    /// 枠の中なら常に当たるようにすると、**中身の隙間を押しても
    /// スクロール領域が取ってしまい**、外側のパネルに届かなくなります。
    /// 車輪は当たり判定を通らない……のではなく
    /// [`Gui::hit_test`](crate::gui::Gui::hit_test) を通るので、
    /// 枠の中はすべて受け取ります。細かい切り分けは
    /// [`ScrollArea::on_event`] で [`EventResult::Ignored`] を返して行います。
    fn hit_test(&self, local: Point, size: Size) -> bool {
        Rect::from_origin_size(Point::ZERO, size).contains(local)
    }

    fn behavior(&self) -> Behavior {
        Behavior::SURFACE
            // 枠の外へ出た中身は見せない。当たりも取らせない。
            .clips_children(true)
            // つまみを掴んだまま枠の外へ出ても離さない。
            .captures_pointer(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Color;
    use crate::gui::container::Stack;
    use crate::gui::context::NoTextMeasure;
    use crate::gui::event::{InputEvent, ScrollDelta};
    use crate::gui::geometry::Insets;
    use crate::gui::id::WidgetId;
    use crate::gui::theme::{Metrics, StateKey, Style, Theme};
    use crate::gui::widget::Behavior as WidgetBehavior;
    use crate::gui::Gui;

    const VIEWPORT: Size = Size {
        width: 200.0,
        height: 100.0,
    };

    /// 決まった大きさを言うだけの中身。
    struct Block {
        size: Size,
        interactive: bool,
    }

    impl Block {
        fn boxed(width: f32, height: f32) -> Box<dyn Widget> {
            Box::new(Self {
                size: Size::new(width, height),
                interactive: false,
            })
        }

        fn interactive(width: f32, height: f32) -> Box<dyn Widget> {
            Box::new(Self {
                size: Size::new(width, height),
                interactive: true,
            })
        }
    }

    impl Widget for Block {
        fn measure(&mut self, _: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
            // 直交する向きは渡された下限に従う（伸ばす設定を見るため）。
            constraints.constrain(self.size)
        }

        fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
            painter.rect(context.rect(), Color::WHITE);
        }

        fn behavior(&self) -> WidgetBehavior {
            if self.interactive {
                WidgetBehavior::SURFACE
            } else {
                WidgetBehavior::DECORATION
            }
        }
    }

    fn theme() -> Theme {
        let mut theme = Theme::new();

        theme.set_metrics(Metrics {
            spacing: 8.0,
            text_size: 10.0,
            control_height: 32.0,
            // 行の高さ 20。車輪 1 刻み = 20 px になる。
            line_height_factor: 2.0,
        });

        theme.set(
            Role::ACCENT,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x202020))
                .foreground(Color::hex(0x888888)),
        );
        theme.set(
            Role::ACCENT,
            StateKey::Hovered,
            Style::BARE
                .background(Color::hex(0x202020))
                .foreground(Color::hex(0xbbbbbb)),
        );
        theme.set(
            Role::ACCENT,
            StateKey::Pressed,
            Style::BARE
                .background(Color::hex(0x202020))
                .foreground(Color::hex(0xffffff)),
        );

        theme
    }

    /// `ScrollArea` を画面いっぱいに置いた `Gui`。中身は 1 つ。
    fn with_content(area: ScrollArea, content: Box<dyn Widget>) -> (Gui, WidgetId, WidgetId) {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let id = gui.set_root(Box::new(area));
        let child = gui.tree_mut().add_child(id, content);

        (gui, id, child)
    }

    fn laid_out(gui: &mut Gui) {
        gui.layout(VIEWPORT, &NoTextMeasure);
    }

    fn area(gui: &Gui, id: WidgetId) -> &ScrollArea {
        gui.tree().get_as::<ScrollArea>(id).expect("ScrollArea")
    }

    fn wheel(gui: &mut Gui, at: Point, lines: f32) {
        gui.handle_input(InputEvent::Scrolled {
            position: at,
            delta: ScrollDelta::Lines { x: 0.0, y: lines },
        });
    }

    #[test]
    fn content_is_measured_without_a_limit_on_the_scroll_axis() {
        let (mut gui, id, child) =
            with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));

        laid_out(&mut gui);

        // 枠は 100 しかないが、中身は 500 のまま測られている。
        assert_eq!(gui.tree().bounds(child).height, 500.0);
        assert_eq!(area(&gui, id).content_length(), 500.0);
        assert_eq!(area(&gui, id).max_offset(), 400.0);
        assert!(area(&gui, id).is_scrollable());
    }

    #[test]
    fn the_scrollbar_width_is_taken_out_of_the_content() {
        let (mut gui, _, child) = with_content(
            ScrollArea::vertical().scrollbar_width(12.0),
            Block::boxed(10.0, 500.0),
        );

        laid_out(&mut gui);

        // 200 - 12 = 188。伸ばす設定が既定なので幅いっぱい。
        assert_eq!(gui.tree().bounds(child).width, 188.0);
    }

    #[test]
    fn content_that_fits_is_not_scrollable() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 40.0));

        laid_out(&mut gui);

        assert!(!area(&gui, id).is_scrollable());
        assert_eq!(area(&gui, id).max_offset(), 0.0);
        assert!(area(&gui, id).thumb().is_none(), "棒は出さない");
    }

    #[test]
    fn scrolling_moves_the_child_to_a_negative_position() {
        let (mut gui, id, child) =
            with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));

        laid_out(&mut gui);
        assert_eq!(gui.tree().bounds(child).y, 0.0);

        // 車輪を 2 刻み下へ。行の高さ 20 なので 40 px。
        wheel(&mut gui, Point::new(50.0, 50.0), -2.0);
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 40.0);
        assert_eq!(gui.tree().bounds(child).y, -40.0, "置き場所そのものが動く");
    }

    #[test]
    fn a_scrolled_child_is_no_longer_hit() {
        let (mut gui, _, child) =
            with_content(ScrollArea::vertical(), Block::interactive(50.0, 500.0));

        laid_out(&mut gui);

        // 中身の上。
        assert_eq!(gui.hit_test(Point::new(10.0, 10.0)), child);

        // 枠の外（下）は当たらない。
        assert_eq!(gui.hit_test(Point::new(10.0, 150.0)), WidgetId::NONE);
    }

    #[test]
    fn scrolling_past_the_end_clamps() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 150.0));

        laid_out(&mut gui);

        // 送れるのは 50 まで。
        wheel(&mut gui, Point::new(50.0, 50.0), -100.0);
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 50.0);

        // 先頭より手前へは行かない。
        wheel(&mut gui, Point::new(50.0, 50.0), 100.0);
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 0.0);
    }

    #[test]
    fn at_the_end_the_wheel_goes_to_the_parent() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        // 外に縦、中にも縦。**内側には丈を決めてやる必要がある**
        // （外側は上限なしで測るので、そのままでは内側が送らない）。
        let outer = gui.set_root(Box::new(ScrollArea::vertical()));
        let list = gui.tree_mut().add_child(outer, Box::new(Stack::column()));

        let inner = gui
            .tree_mut()
            .add_child(list, Box::new(ScrollArea::vertical()));
        gui.tree_mut().add_child(inner, Block::boxed(50.0, 500.0));

        // 外の中身を長くするための相棒。
        gui.tree_mut().add_child(list, Block::boxed(50.0, 400.0));

        gui.tree_mut()
            .get_as_mut::<Stack>(list)
            .unwrap()
            .set_length(inner, crate::gui::layout::Length::Fixed(60.0));

        laid_out(&mut gui);

        assert!(area(&gui, inner).is_scrollable(), "丈を決めたので送れる");

        let at = Point::new(50.0, 50.0);

        // 内側が端に着くまでは内側が受け取る。
        wheel(&mut gui, at, -100.0);
        laid_out(&mut gui);

        let inner_offset = area(&gui, inner).offset();
        assert!(inner_offset > 0.0);
        assert_eq!(area(&gui, outer).offset(), 0.0, "まだ外は動かない");

        // 内側は端。ここからは外が受け取る。
        wheel(&mut gui, at, -100.0);
        laid_out(&mut gui);

        assert_eq!(area(&gui, inner).offset(), inner_offset, "内側は動かない");
        assert!(area(&gui, outer).offset() > 0.0, "外が続きを受け取る");
    }

    #[test]
    fn the_thumb_shrinks_as_the_content_grows() {
        let (mut gui, short, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 200.0));
        laid_out(&mut gui);
        let short_thumb = area(&gui, short).thumb().unwrap().height;

        let (mut gui, tall, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 2000.0));
        laid_out(&mut gui);
        let tall_thumb = area(&gui, tall).thumb().unwrap().height;

        assert!(short_thumb > tall_thumb, "{short_thumb} > {tall_thumb}");
        // 短すぎると掴めないので下限がある。
        assert_eq!(tall_thumb, DEFAULT_MIN_THUMB);
    }

    #[test]
    fn the_thumb_sits_at_the_end_when_scrolled_to_the_end() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let track = area(&gui, id).track();

        gui.tree_mut()
            .get_as_mut::<ScrollArea>(id)
            .unwrap()
            .scroll_to_end();
        gui.tree_mut().request_layout(id);
        laid_out(&mut gui);

        let thumb = area(&gui, id).thumb().unwrap();

        assert_eq!(thumb.bottom(), track.bottom(), "末尾では底に着く");
    }

    #[test]
    fn dragging_the_thumb_scrolls() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let thumb = area(&gui, id).thumb().unwrap();
        let grab = thumb.center();

        gui.handle_input(InputEvent::PointerPressed {
            position: grab,
            button: PointerButton::Primary,
        });

        assert!(area(&gui, id).is_dragging());
        assert_eq!(gui.pointer_capture(), id);

        // つまみを 20 px 下へ。
        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(grab.x, grab.y + 20.0),
        });
        laid_out(&mut gui);

        let offset = area(&gui, id).offset();

        // つまみの移れる幅より中身のほうが長いので、送る量は 20 より大きい。
        assert!(offset > 20.0, "{offset}");
        assert!(offset <= area(&gui, id).max_offset());

        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(grab.x, grab.y + 20.0),
            button: PointerButton::Primary,
        });

        assert!(!area(&gui, id).is_dragging());
        assert_eq!(gui.pointer_capture(), WidgetId::NONE);
    }

    #[test]
    fn dragging_the_thumb_outside_keeps_working() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let grab = area(&gui, id).thumb().unwrap().center();

        gui.handle_input(InputEvent::PointerPressed {
            position: grab,
            button: PointerButton::Primary,
        });

        // 枠の外まで引く。掴んでいるので届き続ける。
        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(grab.x + 500.0, grab.y + 40.0),
        });
        laid_out(&mut gui);

        assert!(area(&gui, id).offset() > 0.0, "外へ出ても送れる");
    }

    #[test]
    fn pressing_the_track_jumps_a_page() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let track = area(&gui, id).track();

        // つまみより下の通り道を押す。
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(track.center().x, track.bottom() - 2.0),
            button: PointerButton::Primary,
        });
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 100.0, "枠 1 つぶん飛ぶ");
        assert!(!area(&gui, id).is_dragging(), "通り道では掴まない");
    }

    #[test]
    fn the_thumb_changes_colour_when_hovered() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let thumb_colour = |gui: &mut Gui| {
            let list = gui.paint();

            // 通り道 → つまみ の順に出る。
            let thumb = list
                .commands_of(id)
                .last()
                .expect("つまみ")
                .primitive
                .clone();

            match thumb {
                crate::gui::painter::Primitive::Rect { color, .. } => color,
                other => panic!("つまみは塗った矩形: {other:?}"),
            }
        };

        let idle = thumb_colour(&mut gui);

        let at = area(&gui, id).thumb().unwrap().center();
        gui.handle_input(InputEvent::PointerMoved { position: at });

        let hovered = thumb_colour(&mut gui);

        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });

        let pressed = thumb_colour(&mut gui);

        assert_eq!(idle, Color::hex(0x888888));
        assert_eq!(hovered, Color::hex(0xbbbbbb));
        assert_eq!(pressed, Color::hex(0xffffff));
    }

    #[test]
    fn the_scrollbar_paints_over_the_content() {
        let (mut gui, id, child) =
            with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let list = gui.paint();
        let owners: Vec<_> = list.commands().iter().map(|c| c.owner).collect();

        // 中身 → 通り道 → つまみ。棒は `paint_foreground` なので後。
        assert_eq!(owners, vec![child, id, id]);
    }

    #[test]
    fn content_is_clipped_to_the_viewport() {
        let (mut gui, _, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let list = gui.paint();
        let clip = list.commands()[0].clip.expect("中身は切り抜かれる");

        assert_eq!(clip.rect, Rect::from_origin_size(Point::ZERO, VIEWPORT));
    }

    #[test]
    fn shrinking_the_content_pulls_the_offset_back() {
        let (mut gui, id, child) =
            with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        gui.tree_mut()
            .get_as_mut::<ScrollArea>(id)
            .unwrap()
            .scroll_to_end();
        gui.tree_mut().request_layout(id);
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 400.0);

        // 中身を縮める。送りすぎのままだと、空白が見えてしまう。
        gui.tree_mut().remove(child);
        gui.tree_mut().add_child(id, Block::boxed(50.0, 120.0));
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).max_offset(), 20.0);
        assert_eq!(area(&gui, id).offset(), 20.0, "端まで引き戻す");
    }

    #[test]
    fn scroll_into_view_only_moves_when_it_has_to() {
        let (mut gui, id, _) = with_content(ScrollArea::vertical(), Block::boxed(50.0, 500.0));
        laid_out(&mut gui);

        let scroll = |gui: &mut Gui, rect: Rect| {
            let moved = gui
                .tree_mut()
                .get_as_mut::<ScrollArea>(id)
                .unwrap()
                .scroll_into_view(rect);

            gui.tree_mut().request_layout(id);
            gui.layout(VIEWPORT, &NoTextMeasure);

            moved
        };

        // すでに見えている。
        assert!(!scroll(&mut gui, Rect::new(0.0, 10.0, 10.0, 10.0)));
        assert_eq!(area(&gui, id).offset(), 0.0);

        // 奥にある。下端が見えるところまで送る。
        assert!(scroll(&mut gui, Rect::new(0.0, 300.0, 10.0, 20.0)));
        assert_eq!(area(&gui, id).offset(), 220.0, "320 - 100");

        // 手前に戻る。
        assert!(scroll(&mut gui, Rect::new(0.0, 50.0, 10.0, 10.0)));
        assert_eq!(area(&gui, id).offset(), 50.0);
    }

    #[test]
    fn a_horizontal_area_scrolls_sideways() {
        let (mut gui, id, child) =
            with_content(ScrollArea::horizontal(), Block::boxed(800.0, 50.0));

        laid_out(&mut gui);

        assert_eq!(area(&gui, id).max_offset(), 600.0, "800 - 200");

        // 通り道は下端。
        let track = area(&gui, id).track();
        assert_eq!(track.y, VIEWPORT.height - DEFAULT_SCROLLBAR_WIDTH);
        assert_eq!(track.width, VIEWPORT.width);

        gui.handle_input(InputEvent::Scrolled {
            position: Point::new(50.0, 50.0),
            delta: ScrollDelta::Pixels { x: -30.0, y: 0.0 },
        });
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 30.0);
        assert_eq!(gui.tree().bounds(child).x, -30.0);
    }

    #[test]
    fn children_stack_along_the_axis() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let id = gui.set_root(Box::new(ScrollArea::vertical()));
        let first = gui.tree_mut().add_child(id, Block::boxed(50.0, 60.0));
        let second = gui.tree_mut().add_child(id, Block::boxed(50.0, 80.0));

        laid_out(&mut gui);

        assert_eq!(area(&gui, id).content_length(), 140.0);
        assert_eq!(gui.tree().bounds(first).y, 0.0);
        assert_eq!(gui.tree().bounds(second).y, 60.0);
    }

    #[test]
    fn a_stack_inside_gets_the_gaps_right() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let id = gui.set_root(Box::new(ScrollArea::vertical()));
        let list = gui.tree_mut().add_child(
            id,
            Box::new(Stack::column().gap(4.0).padding(Insets::all(6.0))),
        );

        for _ in 0..10 {
            gui.tree_mut().add_child(list, Block::boxed(50.0, 30.0));
        }

        laid_out(&mut gui);

        // 30 × 10 + 4 × 9 + 6 × 2 = 348。
        assert_eq!(area(&gui, id).content_length(), 348.0);
        assert_eq!(gui.tree().bounds(list).height, 348.0);
    }

    #[test]
    fn a_press_outside_the_bar_is_left_to_the_parent() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        // 背景のパネルが押下を待っている。
        let root = gui.set_root(Box::new(
            Stack::column().behavior(WidgetBehavior::SURFACE),
        ));
        let id = gui
            .tree_mut()
            .add_child(root, Box::new(ScrollArea::vertical()));
        gui.tree_mut().add_child(id, Block::boxed(50.0, 500.0));

        laid_out(&mut gui);

        // 棒の外（中身の上）を押す。中身は飾りなので取らない。
        let at = Point::new(20.0, 20.0);
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });
        gui.handle_input(InputEvent::PointerReleased {
            position: at,
            button: PointerButton::Primary,
        });

        let actions = gui.drain_actions();

        // 当たったのはスクロール領域だが、受け取らなかったので
        // `Clicked` は `Gui` が押下先（領域）に積む。
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].widget, id);
        assert_eq!(area(&gui, id).offset(), 0.0, "押しただけでは送らない");
    }

    #[test]
    fn no_scrollbar_means_no_reserved_width() {
        let (mut gui, id, child) = with_content(
            ScrollArea::vertical().scrollbar_width(0.0),
            Block::boxed(10.0, 500.0),
        );

        laid_out(&mut gui);

        assert_eq!(gui.tree().bounds(child).width, 200.0, "幅を取らない");
        assert!(area(&gui, id).track().is_empty());
        assert!(area(&gui, id).thumb().is_none());

        // 車輪では送れる。
        wheel(&mut gui, Point::new(50.0, 50.0), -1.0);
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 20.0);
    }

    #[test]
    fn a_custom_step_overrides_the_line_height() {
        let (mut gui, id, _) = with_content(
            ScrollArea::vertical().step(7.0),
            Block::boxed(50.0, 500.0),
        );

        laid_out(&mut gui);
        wheel(&mut gui, Point::new(50.0, 50.0), -3.0);
        laid_out(&mut gui);

        assert_eq!(area(&gui, id).offset(), 21.0, "7 × 3");
    }

    #[test]
    fn not_stretching_lets_the_content_keep_its_width() {
        let (mut gui, _, child) = with_content(
            ScrollArea::vertical().stretch_cross(false),
            Block::boxed(40.0, 500.0),
        );

        laid_out(&mut gui);

        assert_eq!(gui.tree().bounds(child).width, 40.0);
    }
}
